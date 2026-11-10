use async_trait::async_trait;
use secrecy::ExposeSecret;
use serenity::all::{Context, FullEvent, Message, UserId};
use tracing::debug;
use zayden_app::entitlement::Tier;
use zayden_app::state::AppState;
use zayden_core::{
    EventCtx,
    EventKind,
    HandlerError,
    Module,
    ModuleCtx,
    ModuleError,
    OverlapError,
    RegistryBuilder,
    as_i64,
    require,
    server_tier,
};

use crate::chat::{Message as ChatMessage, Role, strip_speaker_prefix};
use crate::openai::AiClient;
use crate::persona::Persona;

pub struct AiModule {
    persona: Persona,
    world: &'static str,
    user_id: UserId,
}

impl AiModule {
    pub fn from_config(
        cx: &ModuleCtx<'_>,
        persona: Persona,
        world: &'static str,
    ) -> Result<Self, ModuleError> {
        require("ai", "AI_PROVIDER_KEY", cx.config.ai_provider_key.as_ref())?;
        Ok(Self { persona, world, user_id: cx.me.user_id })
    }
}

#[async_trait]
impl Module for AiModule {
    fn id(&self) -> &'static str {
        "ai"
    }

    fn register(&self, _b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        Ok(())
    }

    fn events(&self) -> &'static [EventKind] {
        &[EventKind::Message]
    }

    async fn on_event(
        &self,
        cx: &EventCtx<'_>,
        ev: &FullEvent,
    ) -> Result<(), HandlerError> {
        let FullEvent::Message { new_message, .. } = ev else {
            return Ok(());
        };

        if new_message.author.bot() {
            return Ok(());
        }

        self.run(cx.ctx, new_message, &cx.app).await?;
        Ok(())
    }
}

struct ChatParams<'a> {
    model: &'a str,
    max_tokens: u32,
    word_limit: u32,
}

impl<'a> ChatParams<'a> {
    const FREE_MAX_TOKENS: u32 = 1000;
    const FREE_WORD_LIMIT: u32 = 100;
    const PRO_MAX_TOKENS: u32 = 2000;
    const PRO_WORD_LIMIT: u32 = 300;

    fn for_tier(app: &'a AppState, tier: Tier) -> Self {
        match tier {
            Tier::Free => Self {
                model: &app.ai_model,
                max_tokens: Self::FREE_MAX_TOKENS,
                word_limit: Self::FREE_WORD_LIMIT,
            },
            Tier::Pro | Tier::Ultra => Self {
                model: &app.ai_model_pro,
                max_tokens: Self::PRO_MAX_TOKENS,
                word_limit: Self::PRO_WORD_LIMIT,
            },
        }
    }
}

impl AiModule {
    fn is_own(message: &Message, me: UserId) -> bool {
        message.author.bot() && message.author.id == me
    }

    fn process_referenced_messages(
        msg: &Message,
        me: UserId,
    ) -> Vec<(Role, String)> {
        let mut contents = Vec::new();

        if let Some(referenced_message) = &msg.referenced_message {
            contents.push((
                if Self::is_own(referenced_message, me) {
                    Role::Assistant
                } else {
                    Role::User
                },
                Self::attributed_content(referenced_message, me),
            ));

            let nested_contents =
                Self::process_referenced_messages(referenced_message, me);
            contents.extend(nested_contents);
        }

        contents
    }

    fn attributed_content(message: &Message, me: UserId) -> String {
        let parsed = Self::parse_mentions(message);
        let content = parsed.trim();

        if Self::is_own(message, me) {
            content.to_owned()
        } else {
            format!("{}: {content}", message.author.display_name())
        }
    }

    fn speakers<'a>(
        name: &'static str,
        message: &'a Message,
        me: UserId,
    ) -> Vec<&'a str> {
        let mut names = vec![name];
        let mut next = Some(message);

        while let Some(message) = next {
            if !Self::is_own(message, me) {
                let name = message.author.display_name();

                if !names.contains(&name) {
                    names.push(name);
                }
            }

            next = message.referenced_message.as_deref();
        }

        names
    }

    fn parse_mentions(message: &Message) -> String {
        let mut parsed_content = message.content.to_string();

        for mention in &message.mentions {
            let mention_tag = format!("<@{}>", mention.id);

            if mention.bot() {
                parsed_content = parsed_content.replace(&mention_tag, "");
                continue;
            }

            parsed_content =
                parsed_content.replace(&mention_tag, mention.display_name());
        }

        parsed_content
    }

    async fn reply(
        &self,
        ctx: &Context,
        message: &Message,
        api_key: &str,
        endpoint: &str,
        params: &ChatParams<'_>,
    ) -> Result<(), HandlerError> {
        let me = self.user_id;

        let mut messages = vec![ChatMessage::new(
            Role::System,
            self.persona.system_prompt(self.world, params.word_limit),
        )];

        let mut history = Self::process_referenced_messages(message, me);
        history.reverse();

        for (role, content) in history {
            messages.push(ChatMessage::new(role, content));
        }
        messages.push(ChatMessage::new(
            Role::User,
            Self::attributed_content(message, me),
        ));

        let client = AiClient::new(api_key, endpoint, params.model)?;
        let text = client.chat(messages, params.max_tokens, None).await?;

        let speakers = Self::speakers(self.persona.name, message, me);

        message.reply(&ctx.http, strip_speaker_prefix(&text, &speakers)).await?;
        Ok(())
    }

    async fn run(
        &self,
        ctx: &Context,
        message: &Message,
        app: &AppState,
    ) -> Result<(), HandlerError> {
        let Some(guild_id) = message.guild_id else {
            debug!(channel_id = %message.channel_id, "message is not in a guild; ignoring");
            return Ok(());
        };

        let settings = app.settings.ai.get(as_i64(guild_id.get())).await?;

        if !settings.responds_in(as_i64(message.channel_id.get())) {
            debug!(
                guild_id = %guild_id,
                channel_id = %message.channel_id,
                enabled = settings.enabled,
                "AI is off for this channel; ignoring"
            );
            return Ok(());
        }

        if message
            .referenced_message
            .as_ref()
            .is_some_and(|msg| msg.content.is_empty())
        {
            debug!(
                channel_id = %message.channel_id,
                "referenced message has no content; ignoring"
            );
            return Ok(());
        }

        if !message.mentions_me(ctx).await.unwrap_or(false) {
            debug!(
                channel_id = %message.channel_id,
                author_id = %message.author.id,
                "message does not mention the bot; ignoring"
            );
            return Ok(());
        }

        let author_tier = app.entitlements.user_tier(message.author.id.get()).await;
        let server_tier = server_tier(&ctx.http, &app.entitlements, guild_id).await;
        let tier = author_tier.max(server_tier);

        let params = ChatParams::for_tier(app, tier);
        debug!(
            author_id = %message.author.id,
            %guild_id,
            author_tier = author_tier.as_str(),
            server_tier = server_tier.as_str(),
            tier = tier.as_str(),
            model = params.model,
            "generating AI reply"
        );

        if let Err(e) = self
            .reply(
                ctx,
                message,
                app.ai_provider_key.expose_secret(),
                &app.ai_api_endpoint,
                &params,
            )
            .await
        {
            tracing::error!(error = ?e, channel_id = %message.channel_id, "AI reply failed");
        }

        Ok(())
    }
}

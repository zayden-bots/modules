use futures::StreamExt;
use serenity::all::{CreateComponent, CreateTextDisplay};
use tracing::error;
use zayden_core::announce::{allowed_roles, escape, render};
use zayden_core::limits::message::COMPONENTS_TEXT as MAX_TEXT;
use zayden_core::{CronCtx, Post};

use crate::db::announce::{self, PatchAnnounceRow, PatchTarget};
use crate::db::patches::{self, PatchRow};
use crate::embeds;
use crate::error::Result;

const CONCURRENCY: usize = 5;

pub const DEFAULT_MESSAGE: &str = "{roles} New Deadlock patch: **{title}**";

pub const PLACEHOLDERS: &[(&str, &str)] = &[
    ("{roles}", "the roles to ping"),
    ("{title}", "the patch's title"),
    ("{link}", "the link to the official patch notes"),
];

#[must_use]
pub fn patch_message(row: &PatchAnnounceRow, patch: &PatchRow) -> Post<'static> {
    let title = escape(&patch.title);
    let content = render(
        row.message.as_deref().unwrap_or(DEFAULT_MESSAGE),
        &row.ping_role_ids,
        &[("title", &title), ("link", &patch.link)],
    );

    let card_budget = MAX_TEXT.saturating_sub(content.chars().count());
    let card = embeds::patch_component(patch, card_budget);

    let components = (!content.is_empty())
        .then(|| CreateComponent::TextDisplay(CreateTextDisplay::new(content)))
        .into_iter()
        .chain([card])
        .collect::<Vec<_>>();

    Post::components_v2(components)
        .allowed_mentions(allowed_roles(&row.ping_role_ids))
}

pub async fn announce_ready(cx: &CronCtx) -> Result<usize> {
    let pool = &cx.db;
    let ready = patches::ready(pool).await?;
    if ready.is_empty() {
        return Ok(0);
    }

    let targets = announce::targets(pool).await?;

    for patch in &ready {
        let posts: Vec<_> =
            targets.iter().map(|target| post(cx, target, patch)).collect();

        futures::stream::iter(posts)
            .buffer_unordered(CONCURRENCY)
            .for_each(|()| async {})
            .await;

        patches::mark_announced(pool, &patch.guid).await?;
    }

    Ok(ready.len())
}

async fn post(cx: &CronCtx, target: &PatchTarget, patch: &PatchRow) {
    let message = patch_message(&target.row, patch);

    if let Err(e) =
        cx.webhooks.send(target.row.channel(), target.application_id, &message).await
    {
        error!(
            error = ?e,
            guild_id = target.row.guild_id,
            "deadlock: failed to post patch announcement; it is not retried"
        );
    }
}

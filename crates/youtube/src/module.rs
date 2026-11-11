use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use zayden_app::config::BotConfig;
use zayden_core::{Module, OverlapError, RegistryBuilder};

use crate::YoutubeRuntime;

#[must_use]
pub fn runtime_from_config(config: &BotConfig) -> Option<YoutubeRuntime> {
    let youtube = config.youtube.as_ref()?;
    let Some(google_api_key) = config.google_api_key.as_ref() else {
        tracing::warn!(
            "youtube is configured but GOOGLE_API_KEY is not set; youtube is disabled"
        );
        return None;
    };
    Some(YoutubeRuntime {
        api_key: Arc::from(google_api_key.expose_secret()),
        webhook_uri: Arc::from(youtube.webhook_uri.as_str()),
    })
}

pub struct YoutubeModule;

#[async_trait]
impl Module for YoutubeModule {
    fn id(&self) -> &'static str {
        "youtube"
    }

    fn register(&self, _b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        Ok(())
    }
}

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use zayden_app::config::BotConfig;
use zayden_core::{Module, OverlapError, RegistryBuilder};

use crate::PatreonApp;

#[must_use]
pub fn app_from_config(config: &BotConfig) -> Option<Arc<PatreonApp>> {
    config.patreon.as_ref().map(|p| {
        Arc::new(PatreonApp {
            client_id: p.client_id.clone(),
            client_secret: p.client_secret.expose_secret().to_owned(),
            redirect_uri: p.redirect_uri.clone(),
        })
    })
}

pub struct PatreonModule;

#[async_trait]
impl Module for PatreonModule {
    fn id(&self) -> &'static str {
        "patreon"
    }

    fn register(&self, _b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        Ok(())
    }
}

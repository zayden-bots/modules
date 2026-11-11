mod autocomplete;
mod cache_listener;
mod command;

use std::sync::Arc;

use async_trait::async_trait;
use bungie_api::BungieClient;
pub use cache_listener::spawn_cache_listener;
use sqlx::PgPool;
use zayden_core::error::HandlerError;
use zayden_core::{
    Module,
    ModuleCtx,
    ModuleError,
    OverlapError,
    RegistryBuilder,
    StartCtx,
};

use crate::worker::clients_from_config;

#[async_trait]
pub trait ParentToken: Send + Sync {
    async fn get(&self, db: &PgPool) -> Result<String, HandlerError>;
}

pub struct Destiny2Module {
    bungie_client: Arc<BungieClient>,
    parent_token: Arc<dyn ParentToken>,
}

impl Destiny2Module {
    pub fn from_config(
        cx: &ModuleCtx<'_>,
        parent_token: Arc<dyn ParentToken>,
    ) -> Result<Self, ModuleError> {
        let (bungie_client, _) = clients_from_config(&cx.worker())?;

        Ok(Self { bungie_client, parent_token })
    }
}

#[async_trait]
impl Module for Destiny2Module {
    fn id(&self) -> &'static str {
        "destiny2"
    }

    fn register(&self, b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        b.add_command(command::Destiny2 {
            bungie_client: Arc::clone(&self.bungie_client),
            parent_token: Arc::clone(&self.parent_token),
        })
        .add_autocomplete(autocomplete::Destiny2 {
            bungie_client: Arc::clone(&self.bungie_client),
        });
        Ok(())
    }

    fn on_start(&self, cx: &StartCtx<'_>) {
        spawn_cache_listener(Arc::clone(&cx.app));
    }
}

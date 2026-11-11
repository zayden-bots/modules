mod autocomplete;
mod command;

use std::sync::Arc;

use async_trait::async_trait;
use zayden_core::{Module, ModuleCtx, OverlapError, RegistryBuilder};

use crate::client::MarathonClient;
use crate::worker::client_from_config;

pub struct MarathonModule {
    client: Arc<MarathonClient>,
}

impl MarathonModule {
    #[must_use]
    pub fn from_config(cx: &ModuleCtx<'_>) -> Self {
        Self { client: client_from_config(&cx.worker()) }
    }
}

#[async_trait]
impl Module for MarathonModule {
    fn id(&self) -> &'static str {
        "marathon"
    }

    fn register(&self, b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        b.add_command(command::Marathon { client: Arc::clone(&self.client) })
            .add_autocomplete(autocomplete::Marathon {
                client: Arc::clone(&self.client),
            });
        Ok(())
    }
}

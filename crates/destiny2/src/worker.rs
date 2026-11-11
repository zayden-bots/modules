use std::sync::Arc;

use bungie_api::{BungieClient, BungieClientBuilder};
use secrecy::{ExposeSecret, SecretString};
use zayden_core::{
    ModuleError,
    WorkerJobResult,
    WorkerModule,
    WorkerModuleCtx,
    require,
};

use crate::endgame_analysis::EndgameAnalysisSheetCron;

pub struct Destiny2Worker {
    bungie_client: Arc<BungieClient>,
    google_api_key: SecretString,
}

impl Destiny2Worker {
    pub fn from_config(cx: &WorkerModuleCtx<'_>) -> Result<Self, ModuleError> {
        let (bungie_client, google_api_key) = clients_from_config(cx)?;
        Ok(Self { bungie_client, google_api_key })
    }
}

pub(crate) fn clients_from_config(
    cx: &WorkerModuleCtx<'_>,
) -> Result<(Arc<BungieClient>, SecretString), ModuleError> {
    let bungie_api_key =
        require("destiny2", "BUNGIE_API_KEY", cx.config.bungie_api_key.as_ref())?;
    let google_api_key =
        require("destiny2", "GOOGLE_API_KEY", cx.config.google_api_key.as_ref())?;
    let bungie_client =
        BungieClientBuilder::new(bungie_api_key.expose_secret().to_owned())
            .build()
            .map_err(|e| ModuleError::init("destiny2", e))?;

    Ok((Arc::new(bungie_client), google_api_key.clone()))
}

impl WorkerModule for Destiny2Worker {
    fn id(&self) -> &'static str {
        "destiny2"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        vec![EndgameAnalysisSheetCron::worker_job(
            Arc::clone(&self.bungie_client),
            self.google_api_key.expose_secret().to_owned(),
        )]
    }
}

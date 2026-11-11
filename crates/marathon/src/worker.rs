use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};
use zayden_core::{WorkerJobResult, WorkerModule, WorkerModuleCtx};

use crate::client::MarathonClient;
use crate::cron::{MarathonAnnounceCron, MarathonNewsCron};

pub(crate) fn client_from_config(cx: &WorkerModuleCtx<'_>) -> Arc<MarathonClient> {
    Arc::new(MarathonClient::new(
        cx.app.http.clone(),
        cx.config.flaresolverr_url.clone(),
    ))
}

pub struct MarathonWorker {
    client: Arc<MarathonClient>,
    http: reqwest::Client,
    bungie_api_key: Option<SecretString>,
}

impl MarathonWorker {
    #[must_use]
    pub fn from_config(cx: &WorkerModuleCtx<'_>) -> Self {
        Self {
            client: client_from_config(cx),
            http: cx.app.http.clone(),
            bungie_api_key: cx.config.bungie_api_key.clone(),
        }
    }
}

impl WorkerModule for MarathonWorker {
    fn id(&self) -> &'static str {
        "marathon"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        vec![
            MarathonAnnounceCron::worker_job(Arc::clone(&self.client)),
            MarathonNewsCron::worker_job(
                self.http.clone(),
                self.bungie_api_key.as_ref().map(|k| k.expose_secret().to_owned()),
            ),
        ]
    }
}

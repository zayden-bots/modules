use std::sync::Arc;

use zayden_core::{CronCtx, WorkerJobResult, WorkerModule};

use crate::listener::spawn_patreon_listener;
use crate::{PatreonApp, PatreonPollCron};

pub struct PatreonWorker {
    http: reqwest::Client,
    app: Option<Arc<PatreonApp>>,
}

impl PatreonWorker {
    #[must_use]
    pub const fn new(http: reqwest::Client, app: Option<Arc<PatreonApp>>) -> Self {
        Self { http, app }
    }
}

impl WorkerModule for PatreonWorker {
    fn id(&self) -> &'static str {
        "patreon"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        self.app.as_ref().map_or_else(Vec::new, |app| {
            vec![PatreonPollCron::worker_job(self.http.clone(), Arc::clone(app))]
        })
    }

    fn on_start(&self, cx: &CronCtx) {
        spawn_patreon_listener(cx.clone());
    }
}

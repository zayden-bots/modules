use std::sync::Arc;

use zayden_core::{WorkerJobResult, WorkerModule};

use crate::{
    JellyfinGameRoundReaperCron,
    JellyfinIndexRefreshCron,
    JellyfinRollupCron,
    JellyfinRuntime,
};

pub struct JellyfinWorker {
    runtime: Option<Arc<JellyfinRuntime>>,
}

impl JellyfinWorker {
    #[must_use]
    pub const fn new(runtime: Option<Arc<JellyfinRuntime>>) -> Self {
        Self { runtime }
    }
}

impl WorkerModule for JellyfinWorker {
    fn id(&self) -> &'static str {
        "jellyfin"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        self.runtime.as_ref().map_or_else(Vec::new, |runtime| {
            vec![
                JellyfinIndexRefreshCron::worker_job(Arc::clone(runtime)),
                JellyfinRollupCron::worker_job(Arc::clone(runtime)),
                JellyfinGameRoundReaperCron::worker_job(),
            ]
        })
    }
}

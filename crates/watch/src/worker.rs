use std::sync::Arc;

use jellyfin::runtime::JellyfinRuntime;
use zayden_core::{WorkerJobResult, WorkerModule};

use crate::JellyfinPartyReaperCron;

pub struct WatchWorker {
    runtime: Option<Arc<JellyfinRuntime>>,
}

impl WatchWorker {
    #[must_use]
    pub const fn new(runtime: Option<Arc<JellyfinRuntime>>) -> Self {
        Self { runtime }
    }
}

impl WorkerModule for WatchWorker {
    fn id(&self) -> &'static str {
        "watch"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        self.runtime.as_ref().map_or_else(Vec::new, |runtime| {
            vec![
                JellyfinPartyReaperCron::worker_job(Arc::clone(runtime)),
                JellyfinPartyReaperCron::reconcile_job(Arc::clone(runtime)),
            ]
        })
    }
}

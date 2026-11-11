use std::sync::Arc;

use zayden_core::{ModuleError, WorkerJobResult, WorkerModule, WorkerModuleCtx};

use crate::DeadlockApi;
use crate::cron::{
    DeadlockBriefingCron,
    DeadlockCacheGcCron,
    DeadlockLeaderboardCron,
    DeadlockMatchCron,
    DeadlockPatchCron,
    DeadlockRankCron,
};

pub(crate) fn api_from_config(
    cx: &WorkerModuleCtx<'_>,
) -> Result<Arc<DeadlockApi>, ModuleError> {
    DeadlockApi::new(
        cx.app.http.clone(),
        cx.app.db.clone(),
        cx.config.deadlock_api_key.as_ref(),
    )
    .map(Arc::new)
    .map_err(|e| ModuleError::init("deadlock", e))
}

pub struct DeadlockWorker {
    api: Arc<DeadlockApi>,
}

impl DeadlockWorker {
    pub fn from_config(cx: &WorkerModuleCtx<'_>) -> Result<Self, ModuleError> {
        Ok(Self { api: api_from_config(cx)? })
    }
}

impl WorkerModule for DeadlockWorker {
    fn id(&self) -> &'static str {
        "deadlock"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        vec![
            DeadlockPatchCron::cron_job(Arc::clone(&self.api)),
            DeadlockMatchCron::cron_job(Arc::clone(&self.api)),
            DeadlockRankCron::cron_job(Arc::clone(&self.api)),
            DeadlockLeaderboardCron::cron_job(Arc::clone(&self.api)),
            DeadlockBriefingCron::cron_job(),
            DeadlockCacheGcCron::cron_job(),
        ]
    }
}

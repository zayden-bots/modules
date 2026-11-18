use std::sync::Arc;

use deadlock_api::DeadlockClient;
use zayden_core::{ModuleError, WorkerJobResult, WorkerModule, WorkerModuleCtx};

use crate::cron::{
    DeadlockBriefingCron,
    DeadlockCacheGcCron,
    DeadlockLeaderboardCron,
    DeadlockMatchCron,
    DeadlockPatchCron,
    DeadlockRankCron,
    DeadlockReportCron,
};
use crate::{DeadlockApi, report_fetch};

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
    report_client: Arc<DeadlockClient>,
}

impl DeadlockWorker {
    pub fn from_config(cx: &WorkerModuleCtx<'_>) -> Result<Self, ModuleError> {
        let report_client = report_fetch::report_client(
            cx.app.http.clone(),
            cx.config.deadlock_api_key.as_ref(),
        )
        .map(Arc::new)
        .map_err(|e| ModuleError::init("deadlock", e))?;

        Ok(Self { api: api_from_config(cx)?, report_client })
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
            DeadlockReportCron::cron_job(
                Arc::clone(&self.api),
                Arc::clone(&self.report_client),
            ),
            DeadlockRankCron::cron_job(Arc::clone(&self.api)),
            DeadlockLeaderboardCron::cron_job(Arc::clone(&self.api)),
            DeadlockBriefingCron::cron_job(),
            DeadlockCacheGcCron::cron_job(),
        ]
    }
}

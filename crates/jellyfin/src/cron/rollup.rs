use std::sync::Arc;

use tracing::error;
use zayden_core::{CronCtx, WorkerJob, WorkerJobResult};

use crate::runtime::JellyfinRuntime;
use crate::stats::rollup;

pub struct JellyfinRollupCron;

impl JellyfinRollupCron {
    pub fn worker_job(runtime: Arc<JellyfinRuntime>) -> WorkerJobResult {
        WorkerJob::new("jellyfin_playback_rollup", "0 0 4 * * * *").map(|job| {
            job.set_action(move |cx: CronCtx| {
                let runtime = Arc::clone(&runtime);

                async move {
                    if let Err(e) = rollup::run(&runtime, &cx.db).await {
                        error!(error = ?e, "jellyfin: playback rollup failed");
                    }
                }
            })
        })
    }
}

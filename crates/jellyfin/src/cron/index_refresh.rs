use std::sync::Arc;

use tracing::error;
use zayden_core::{CronCtx, WorkerJob, WorkerJobResult};

use crate::index;
use crate::runtime::JellyfinRuntime;

pub struct JellyfinIndexRefreshCron;

impl JellyfinIndexRefreshCron {
    pub fn worker_job(runtime: Arc<JellyfinRuntime>) -> WorkerJobResult {
        WorkerJob::new("jellyfin_index_refresh", "0 15 */6 * * * *").map(|job| {
            job.set_action(move |cx: CronCtx| {
                let runtime = Arc::clone(&runtime);

                async move {
                    if let Err(e) = index::refresh(&runtime, &cx.db).await {
                        error!(error = ?e, "jellyfin: library index refresh failed");
                    }
                }
            })
        })
    }
}

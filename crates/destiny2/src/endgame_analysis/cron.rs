use std::sync::Arc;

use bungie_api::BungieClient;
use zayden_core::{CronCtx, WorkerJob, WorkerJobResult};

use crate::compendium;
use crate::endgame_analysis::sheet::EndgameAnalysisSheet;

pub struct EndgameAnalysisSheetCron;

impl EndgameAnalysisSheetCron {
    pub fn worker_job(
        bungie_client: Arc<BungieClient>,
        google_api_key: String,
    ) -> WorkerJobResult {
        WorkerJob::new("endgame_analysis_sheet_weekly", "0 0 0 * * Mon *").map(
            |job| {
                job.set_action(move |cx: CronCtx| {
                    let bungie_client = Arc::clone(&bungie_client);
                    let google_api_key = google_api_key.clone();
                    async move {
                        let manifest =
                            EndgameAnalysisSheet::item_manifest(&bungie_client)
                                .await;
                        if let Err(e) = EndgameAnalysisSheet::update(
                            &cx.db,
                            &manifest,
                            &google_api_key,
                        )
                        .await
                        {
                            tracing::error!(
                                error = ?e,
                                "endgame_analysis sheet update failed"
                            );
                        }
                        if let Err(e) =
                            compendium::update(&cx.db, &google_api_key).await
                        {
                            tracing::error!(
                                error = ?e,
                                "destiny2 compendium update failed"
                            );
                        }
                    }
                })
            },
        )
    }
}

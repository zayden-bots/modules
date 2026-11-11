use std::sync::Arc;

use zayden_core::{CronCtx, WorkerJobResult, WorkerModule};

use crate::listener::spawn_youtube_listener;
use crate::{YoutubeLeaseCron, YoutubePollCron, YoutubeRuntime};

pub struct YoutubeWorker {
    http: reqwest::Client,
    runtime: Option<YoutubeRuntime>,
}

impl YoutubeWorker {
    #[must_use]
    pub const fn new(
        http: reqwest::Client,
        runtime: Option<YoutubeRuntime>,
    ) -> Self {
        Self { http, runtime }
    }
}

impl WorkerModule for YoutubeWorker {
    fn id(&self) -> &'static str {
        "youtube"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        self.runtime.as_ref().map_or_else(Vec::new, |runtime| {
            vec![
                YoutubePollCron::worker_job(
                    self.http.clone(),
                    Arc::clone(&runtime.api_key),
                ),
                YoutubeLeaseCron::worker_job(
                    self.http.clone(),
                    Arc::clone(&runtime.webhook_uri),
                ),
            ]
        })
    }

    fn on_start(&self, cx: &CronCtx) {
        if let Some(runtime) = self.runtime.clone() {
            spawn_youtube_listener(cx.clone(), runtime);
        }
    }
}

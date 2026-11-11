use std::sync::Arc;

use tokio::sync::broadcast::error::RecvError;
use tracing::warn;
use zayden_app::events::AppEvent;
use zayden_core::CronCtx;

use crate::{YoutubeRuntime, announce};

pub(crate) fn spawn_youtube_listener(cx: CronCtx, runtime: YoutubeRuntime) {
    tokio::spawn(async move {
        let mut rx = cx.app.subscribe();

        loop {
            match rx.recv().await {
                Ok(AppEvent::YoutubeUpload(channel_id)) => {
                    let cx = cx.clone();
                    let api_key = Arc::clone(&runtime.api_key);

                    tokio::spawn(async move {
                        announce::on_ping(
                            &cx.webhooks,
                            &cx.app.http,
                            &cx.db,
                            &api_key,
                            &channel_id,
                        )
                        .await;
                    });
                },
                Ok(
                    AppEvent::ConfigChanged(_)
                    | AppEvent::ModulesChanged { .. }
                    | AppEvent::EntitlementChanged(_)
                    | AppEvent::PatreonPost(_)
                    | AppEvent::HostingPaid(_)
                    | AppEvent::ServingChanged(_)
                    | AppEvent::CustomBotsChanged(_)
                    | AppEvent::LoadoutsChanged
                    | AppEvent::TaskScheduled(_)
                    | AppEvent::DeliveryChanged(_)
                    | AppEvent::WebhookBroken { .. }
                    // The next poll re-reads every channel, so nothing is lost.
                    | AppEvent::Resync,
                ) => {},
                Err(RecvError::Lagged(n)) => {
                    warn!(
                        n,
                        "youtube webhook listener lagged; the poll will catch up"
                    );
                },
                Err(RecvError::Closed) => break,
            }
        }
    });
}

use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, warn};
use zayden_app::events::AppEvent;
use zayden_core::CronCtx;

use crate::announce_pending;

pub(crate) fn spawn_patreon_listener(cx: CronCtx) {
    tokio::spawn(async move {
        let mut rx = cx.app.subscribe();

        loop {
            match rx.recv().await {
                Ok(AppEvent::PatreonPost(post_id)) => {
                    info!(post_id, "patreon: webhook post received, announcing");
                    announce(&cx).await;
                },
                Ok(AppEvent::Resync) => announce(&cx).await,
                Ok(
                    AppEvent::ConfigChanged(_)
                    | AppEvent::ModulesChanged { .. }
                    | AppEvent::EntitlementChanged(_)
                    | AppEvent::YoutubeUpload(_)
                    | AppEvent::HostingPaid(_)
                    | AppEvent::ServingChanged(_)
                    | AppEvent::CustomBotsChanged(_)
                    | AppEvent::LoadoutsChanged
                    | AppEvent::TaskScheduled(_)
                    | AppEvent::DeliveryChanged(_)
                    | AppEvent::WebhookBroken { .. },
                ) => {},
                Err(RecvError::Lagged(n)) => {
                    warn!(
                        n,
                        "patreon webhook listener lagged; the poll will catch up"
                    );
                },
                Err(RecvError::Closed) => {
                    error!("patreon webhook listener stopped: the event bus closed");
                    break;
                },
            }
        }
    });
}

async fn announce(cx: &CronCtx) {
    if let Err(e) = announce_pending(&cx.webhooks, &cx.app.http, &cx.db).await {
        error!(error = ?e, "patreon: failed to announce pending posts");
    }
}

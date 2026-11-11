use std::sync::Arc;

use jiff::Timestamp;
use serenity::all::{ApplicationId, UserId};
use tracing::{error, warn};
use zayden_app::entitlement::EntitlementService;
use zayden_core::{CronCtx, Post, WorkerJob, WorkerJobResult, as_u64, queue_dm};

use crate::sweep::{self, Claimed};
use crate::{HostingRuntime, pricing, store};

const BATCH: i64 = 50;

pub struct HostingReminderCron;

impl HostingReminderCron {
    pub fn worker_job(runtime: Arc<HostingRuntime>) -> WorkerJobResult {
        WorkerJob::new("hosting_reminder", "0 0 * * * * *").map(|job| {
            job.set_action(move |cx: CronCtx| {
                let runtime = Arc::clone(&runtime);
                async move {
                    let days = clamp_days(runtime.config.reminder_days);

                    let due = match sweep::claim_reminders(&cx.db, days, BATCH).await
                    {
                        Ok(due) => due,
                        Err(e) => {
                            error!(error = %e, "hosting reminder sweep failed");
                            return;
                        },
                    };

                    for row in due {
                        remind(&cx, &runtime, &row).await;
                    }
                }
            })
        })
    }
}

pub struct HostingExpireCron;

impl HostingExpireCron {
    pub fn worker_job(
        runtime: Arc<HostingRuntime>,
        entitlements: Arc<EntitlementService>,
    ) -> WorkerJobResult {
        WorkerJob::new("hosting_expire", "0 */5 * * * * *").map(|job| {
            job.set_action(move |cx: CronCtx| {
                let runtime = Arc::clone(&runtime);
                let entitlements = Arc::clone(&entitlements);
                async move {
                    let grace = clamp_days(runtime.config.grace_days);

                    match sweep::convert_included_trials(&cx.db, BATCH).await {
                        Ok(rows) => {
                            for row in rows {
                                notify_converted(&cx, &row).await;
                            }
                        },
                        Err(e) => {
                            error!(error = %e, "hosting trial conversion failed");
                        },
                    }

                    match sweep::claim_expired(&cx.db, grace, BATCH).await {
                        Ok(rows) => {
                            for row in rows {
                                suspend(&cx, &runtime, &row).await;
                            }
                        },
                        Err(e) => error!(error = %e, "hosting expire sweep failed"),
                    }

                    revalidate(&cx, &runtime, &entitlements, grace).await;

                    runtime.unsuspend_due(&cx.db).await;
                }
            })
        })
    }
}

pub struct HostingDeleteCron;

impl HostingDeleteCron {
    pub fn worker_job(runtime: Arc<HostingRuntime>) -> WorkerJobResult {
        WorkerJob::new("hosting_delete", "0 30 * * * * *").map(|job| {
            job.set_action(move |cx: CronCtx| {
                let runtime = Arc::clone(&runtime);
                async move {
                    let due = match sweep::claim_deletions(&cx.db, BATCH).await {
                        Ok(due) => due,
                        Err(e) => {
                            error!(error = %e, "hosting delete sweep failed");
                            return;
                        },
                    };

                    for row in due {
                        delete(&cx, &runtime, &row).await;
                    }
                }
            })
        })
    }
}

async fn remind(cx: &CronCtx, runtime: &HostingRuntime, row: &Claimed) {
    let when = row
        .expires_at()
        .map_or_else(|| "soon".to_owned(), |t| format!("<t:{}:R>", t.as_second()));

    let price = pricing::format_price(i64::from(row.price_cents));
    let kofi = &runtime.config.kofi_url;

    let content = if row.price_cents == 0 {
        format!(
            "Your **{}** server renews {when} against your subscription. \
             Nothing to do — just keep the subscription active.",
            row.game_key
        )
    } else {
        format!(
            "Your **{}** server expires {when}.\nRenew for **{price}/month** at \
             {kofi} and put `{}` in the message so I know which server it is.",
            row.game_key, row.claim_code
        )
    };

    let key = format!("hosting:{}:reminder:{}", row.id, expires_unix(row));
    notify(cx, target(cx, row), row.owner(), key, content).await;
}

async fn notify_converted(cx: &CronCtx, row: &Claimed) {
    let content = format!(
        "Your **{}** trial has ended, and your subscription covers this plan — \
         the server stays up at no extra cost.",
        row.game_key
    );

    let key = format!("hosting:{}:converted", row.id);
    notify(cx, target(cx, row), row.owner(), key, content).await;
}

async fn suspend(cx: &CronCtx, runtime: &HostingRuntime, row: &Claimed) {
    if let Some(server_id) = row.pelican_server_id
        && let Err(e) = runtime.pelican.suspend(server_id).await
    {
        warn!(error = %e, row = row.id, "hosting: panel suspend failed");
    }

    let price = pricing::format_price(i64::from(row.price_cents));
    let kofi = &runtime.config.kofi_url;

    // A trial that was never paid for gets no grace, so it is about to go —
    // promising N days here would be a straight lie.
    let content = if row.has_paid {
        format!(
            "Your **{}** server has been suspended — its paid window ran out.\n\
             It will be deleted in {} days. Renew for **{price}/month** at \
             {kofi} with `{}` in the message and it comes straight back.",
            row.game_key, runtime.config.grace_days, row.claim_code
        )
    } else {
        format!(
            "Your **{}** trial has ended and the server is suspended. It will \
             be deleted shortly. Subscribe for **{price}/month** at {kofi} with \
             `{}` in the message to keep it — do that before it is deleted and \
             the world carries over.",
            row.game_key, row.claim_code
        )
    };

    let key = format!("hosting:{}:suspended:{}", row.id, expires_unix(row));
    notify(cx, target(cx, row), row.owner(), key, content).await;
}

async fn delete(cx: &CronCtx, runtime: &HostingRuntime, row: &Claimed) {
    if let Some(server_id) = row.pelican_server_id
        && let Err(e) = runtime.pelican.delete_server(server_id).await
    {
        // Leaving the row claimed means the lease expires and this retries,
        // rather than the row being marked deleted while the server lives on.
        warn!(error = %e, row = row.id, "hosting: panel delete failed; will retry");
        return;
    }

    if let Err(e) = store::mark_deleted(&cx.db, row.id).await {
        error!(error = %e, row = row.id, "hosting: could not mark row deleted");
        return;
    }

    let content = format!(
        "Your **{}** server has been deleted after its grace period. Its files \
         are gone; `/server host` will set up a fresh one whenever you want.",
        row.game_key
    );

    let key = format!("hosting:{}:deleted", row.id);
    notify(cx, target(cx, row), row.owner(), key, content).await;
}

async fn revalidate(
    cx: &CronCtx,
    runtime: &HostingRuntime,
    entitlements: &EntitlementService,
    grace: i32,
) {
    let rows = match sweep::entitlement_backed(&cx.db, BATCH).await {
        Ok(rows) => rows,
        Err(e) => {
            error!(error = %e, "hosting entitlement revalidation failed");
            return;
        },
    };

    for row in rows {
        let Ok(plan) = row.plan() else {
            warn!(row = row.id, plan = %row.plan, "hosting: unknown plan on row");
            continue;
        };

        let tier = entitlements.user_tier(row.owner().get()).await;
        if pricing::is_included(plan, tier) {
            continue;
        }

        if let Err(e) = sweep::suspend(&cx.db, row.id, grace).await {
            error!(error = %e, row = row.id, "hosting: suspend failed");
            continue;
        }

        suspend(cx, runtime, &row).await;
    }
}

fn target(cx: &CronCtx, row: &Claimed) -> ApplicationId {
    ApplicationId::new(row.dm_application_id.map_or(cx.app.zayden_id, as_u64))
}

fn expires_unix(row: &Claimed) -> i64 {
    row.expires_at().map_or(0, Timestamp::as_second)
}

async fn notify(
    cx: &CronCtx,
    target: ApplicationId,
    user: UserId,
    key: String,
    content: String,
) {
    if let Err(e) = queue_dm(
        &cx.db,
        target,
        user,
        key,
        Timestamp::now(),
        &Post::content(content),
    )
    .await
    {
        warn!(error = %e, %user, "hosting: could not queue the owner DM");
    }
}

fn clamp_days(days: i64) -> i32 {
    i32::try_from(days.clamp(0, 365)).unwrap_or(0)
}

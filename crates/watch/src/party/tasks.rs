use std::sync::Arc;

use jellyfin::runtime::JellyfinRuntime;
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::json;
use serenity::all::Http;
use sqlx::PgPool;
use tracing::error;
use zayden_app::tasks::{NewTask, ScheduledTasks, Task};
use zayden_core::Identity;

use crate::party::lifecycle;
use crate::party::row::PartyRow;

pub const PROVISION_LEAD: i64 = 30;

pub const PROVISION: &str = "watch_party_provision";
pub const ANNOUNCE: &str = "watch_party_announce";
pub const CLEANUP: &str = "watch_party_cleanup";

#[derive(Debug, Serialize, Deserialize)]
pub struct PartyPayload {
    pub party_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnnouncePayload {
    pub party_id: i64,
    pub content: String,
}

#[must_use]
pub fn key_prefix(party_id: i64) -> String {
    format!("party:{party_id}:")
}

#[must_use]
pub fn tasks_for(party: &PartyRow, now: Timestamp) -> Vec<NewTask> {
    let party_id = party.id;
    let prefix = key_prefix(party_id);
    let start = party.starts_at();
    let name = &party.item_name;

    let mut tasks = Vec::with_capacity(6);

    let provision_at = start - Span::new().minutes(PROVISION_LEAD);
    if provision_at > now {
        tasks.push(NewTask {
            kind: PROVISION,
            key: format!("{prefix}provision"),
            run_at: provision_at,
            payload: json!({ "party_id": party_id }),
        });
    }

    let reminders = [
        ("remind_24h", Span::new().hours(24), "starts in 24 hours"),
        ("remind_1h", Span::new().hours(1), "starts in an hour"),
        ("remind_15m", Span::new().minutes(15), "starts in 15 minutes"),
    ];

    for (suffix, offset, phrase) in reminders {
        let at = start - offset;
        if at <= now {
            continue;
        }

        tasks.push(NewTask {
            kind: ANNOUNCE,
            key: format!("{prefix}{suffix}"),
            run_at: at,
            payload: json!({
                "party_id": party_id,
                "content": format!("**{name}** {phrase}."),
            }),
        });
    }

    if start > now {
        tasks.push(NewTask {
            kind: ANNOUNCE,
            key: format!("{prefix}start"),
            run_at: start,
            payload: json!({
                "party_id": party_id,
                "content": format!(
                    "**{name}** starts now. The host should start playback and open \
                     a SyncPlay group from their client — everyone else, join it \
                     from the cast menu."
                ),
            }),
        });
    }

    let cleanup_at = party.cleanup_after();
    if cleanup_at > now {
        tasks.push(NewTask {
            kind: CLEANUP,
            key: format!("{prefix}cleanup"),
            run_at: cleanup_at,
            payload: json!({ "party_id": party_id }),
        });
    }

    tasks
}

#[must_use]
pub fn rebuild_tasks(party: &PartyRow, now: Timestamp) -> Vec<NewTask> {
    let prefix = key_prefix(party.id);
    let payload = json!({ "party_id": party.id });

    if party.cleanup_after() <= now {
        return vec![NewTask {
            kind: CLEANUP,
            key: format!("{prefix}cleanup"),
            run_at: now,
            payload,
        }];
    }

    let mut tasks = tasks_for(party, now);

    let provision_at = party.starts_at() - Span::new().minutes(PROVISION_LEAD);
    if provision_at <= now && party.state == "scheduled" {
        tasks.push(NewTask {
            kind: PROVISION,
            key: format!("{prefix}provision"),
            run_at: now,
            payload,
        });
    }

    tasks
}

pub async fn ensure(
    pool: &PgPool,
    me: Identity,
    party: &PartyRow,
) -> sqlx::Result<()> {
    ScheduledTasks::ensure(
        pool,
        me.application_i64(),
        &rebuild_tasks(party, Timestamp::now()),
    )
    .await
}

pub async fn schedule(
    pool: &PgPool,
    me: Identity,
    party: &PartyRow,
) -> sqlx::Result<()> {
    ScheduledTasks::replace(
        pool,
        me.application_i64(),
        &key_prefix(party.id),
        &tasks_for(party, Timestamp::now()),
    )
    .await
}

pub async fn clear(pool: &PgPool, me: Identity, party_id: i64) -> sqlx::Result<()> {
    ScheduledTasks::cancel(pool, me.application_i64(), &key_prefix(party_id))
        .await?;
    Ok(())
}

pub async fn run(
    http: &Http,
    runtime: &Arc<JellyfinRuntime>,
    pool: &PgPool,
    task: &Task,
) -> Result<(), serde_json::Error> {
    match task.kind.as_str() {
        PROVISION => {
            let PartyPayload { party_id } = task.payload()?;
            if let Err(e) = lifecycle::provision(http, runtime, pool, party_id).await
            {
                error!(error = ?e, party_id, "party provisioning failed");
            }
        },
        ANNOUNCE => {
            if task.is_stale(Timestamp::now()) {
                return Ok(());
            }
            let AnnouncePayload { party_id, content } = task.payload()?;
            lifecycle::announce(http, runtime, pool, party_id, content).await;
        },
        CLEANUP => {
            let PartyPayload { party_id } = task.payload()?;
            if let Err(e) = lifecycle::cleanup(runtime, pool, party_id).await {
                error!(error = ?e, party_id, "party cleanup failed");
            }
        },
        _ => {},
    }

    Ok(())
}

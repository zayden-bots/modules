use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serenity::all::{ApplicationId, Http};
use sqlx::PgPool;
use tracing::warn;
use zayden_app::tasks::{NewTask, PLATFORM_STALE_AFTER, ScheduledTasks, Task};
use zayden_core::{PostPayload, as_i64, deliver_post};

use crate::db::squad_channel;
use crate::error::{DeadlockError, Result};

pub const LEADERBOARD: &str = "deadlock_leaderboard";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardPayload {
    pub guild_id: i64,
    pub post: PostPayload,
}

#[must_use]
pub fn key(guild_id: i64) -> String {
    format!("deadlock:leaderboard:{guild_id}")
}

pub async fn queue(
    pool: &PgPool,
    application_id: ApplicationId,
    guild_id: i64,
    post: PostPayload,
) -> Result<()> {
    let key = key(guild_id);
    let payload =
        serde_json::to_value(LeaderboardPayload { guild_id, post }).map_err(
            |source| DeadlockError::Encode { what: "leaderboard task", source },
        )?;

    let task = NewTask {
        kind: LEADERBOARD,
        key: key.clone(),
        run_at: Timestamp::now(),
        payload,
    };
    ScheduledTasks::replace(pool, as_i64(application_id.get()), &key, &[task])
        .await?;
    Ok(())
}

pub async fn run(http: &Http, pool: &PgPool, task: &Task) -> Result<()> {
    if Timestamp::now().duration_since(task.run_at) > PLATFORM_STALE_AFTER {
        warn!(key = %task.key, "deadlock: dropping a stale leaderboard task");
        return Ok(());
    }

    let LeaderboardPayload { guild_id, post } =
        task.payload().map_err(|source| DeadlockError::Encode {
            what: "leaderboard task payload",
            source,
        })?;

    match deliver_post(http, &post).await {
        Ok(id) => {
            squad_channel::set_leaderboard_message(
                pool,
                guild_id,
                id.get().cast_signed(),
            )
            .await?;
        },
        Err(e) => warn!(
            error = ?e,
            guild_id,
            "deadlock: failed to deliver the squad leaderboard; it is not retried"
        ),
    }
    Ok(())
}

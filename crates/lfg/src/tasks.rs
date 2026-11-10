use futures::future;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serenity::all::{
    Colour,
    CreateEmbed,
    CreateMessage,
    Http,
    Mentionable,
    ThreadId,
};
use sqlx::PgPool;
use tracing::error;
use zayden_app::tasks::{NewTask, ScheduledTasks, Task};
use zayden_core::Identity;

use crate::{Join, PostRow};

pub const REMINDER: &str = "lfg_reminder";

#[derive(Debug, Serialize, Deserialize)]
pub struct ReminderPayload {
    pub thread_id: u64,
}

#[must_use]
pub fn key_prefix(thread: ThreadId) -> String {
    format!("lfg:{thread}:")
}

#[must_use]
pub fn reminders(row: &PostRow, now: Timestamp) -> Vec<NewTask> {
    let thread = row.thread();
    let start = row.start_time.to_jiff();
    let prefix = key_prefix(thread);

    [
        ("week", SignedDuration::from_hours(24 * 7)),
        ("day", SignedDuration::from_hours(24)),
        ("mins_30", SignedDuration::from_mins(30)),
        ("now", SignedDuration::ZERO),
    ]
    .into_iter()
    .filter_map(|(name, lead)| {
        let run_at = start.checked_sub(lead).ok()?;
        if run_at <= now {
            return None;
        }
        let payload =
            serde_json::to_value(ReminderPayload { thread_id: thread.get() })
                .ok()?;
        Some(NewTask {
            kind: REMINDER,
            key: format!("{prefix}{name}"),
            run_at,
            payload,
        })
    })
    .collect()
}

pub async fn schedule(
    pool: &PgPool,
    me: Identity,
    row: &PostRow,
) -> sqlx::Result<()> {
    ScheduledTasks::replace(
        pool,
        me.application_i64(),
        &key_prefix(row.thread()),
        &reminders(row, Timestamp::now()),
    )
    .await
}

pub async fn run(
    http: &Http,
    pool: &PgPool,
    task: &Task,
) -> Result<(), serde_json::Error> {
    if task.is_stale(Timestamp::now()) {
        return Ok(());
    }

    let ReminderPayload { thread_id } = task.payload()?;
    reminder(http, pool.clone(), ThreadId::new(thread_id)).await;
    Ok(())
}

async fn reminder(http: &Http, pool: PgPool, id: ThreadId) {
    let post = match PostRow::get(&pool, id.widen()).await {
        Ok(post) => post,
        // Post deleted
        Err(sqlx::Error::RowNotFound) => return,
        Err(e) => {
            error!(error = ?e, post_id = %id, "lfg reminder: post_row lookup failed");
            return;
        },
    };

    let timestamp = post.start_time.to_jiff();

    let embed = CreateEmbed::new()
        .title(format!("{} - <t:{timestamp}>", post.activity))
        .colour(Colour::BLUE)
        .description(format!(
            "Starting <t:{timestamp}:R>\nThread: {}",
            post.thread().widen().mention()
        ))
        .field(
            "Joined",
            post.fireteam()
                .map(|user| user.mention().to_string())
                .collect::<Vec<_>>()
                .join(" | "),
            false,
        );

    let iter = post
        .fireteam()
        .map(|user| user.dm(http, CreateMessage::new().embed(embed.clone())));

    future::join_all(iter).await;
}

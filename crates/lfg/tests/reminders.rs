use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use lfg::tasks::{ensure, key_prefix, reminders, schedule};
use lfg::{PostBuilder, PostRow};
use serenity::all::{ApplicationId, ThreadId, UserId};
use sqlx::PgPool;
use zayden_app::applications::{AppKind, Slot};
use zayden_app::tasks::ScheduledTasks;
use zayden_core::Identity;

const ME: Identity = Identity {
    application_id: ApplicationId::new(1),
    user_id: UserId::new(2),
    slot: Slot::Zayden,
    kind: AppKind::Default,
};
const THREAD: u64 = 1_099_425_082_890_113_024;

fn post(start: Timestamp) -> PostRow {
    PostBuilder::new(
        UserId::new(3),
        "Vault of Glass",
        start.to_zoned(TimeZone::UTC),
        "Fresh run",
        6,
    )
    .id(ThreadId::new(THREAD))
    .build()
}

#[test]
fn reminders_skip_past_instants() {
    let now = Timestamp::now();
    let row = post(now + SignedDuration::from_hours(2));

    let keys: Vec<_> =
        reminders(&row, now).into_iter().map(|task| task.key).collect();

    let prefix = key_prefix(ThreadId::new(THREAD));
    assert_eq!(keys, vec![format!("{prefix}mins_30"), format!("{prefix}now")]);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn schedule_replaces_previous_reminders(pool: PgPool) {
    let now = Timestamp::now();
    let first = post(now + SignedDuration::from_hours(49));
    let start = now + SignedDuration::from_hours(2);
    let second = post(start);
    let prefix = key_prefix(ThreadId::new(THREAD));

    schedule(&pool, ME, &first).await.unwrap();
    let before = sqlx::query_scalar!(
        "SELECT key FROM scheduled_tasks WHERE application_id = 1"
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(before.contains(&format!("{prefix}day")));

    schedule(&pool, ME, &second).await.unwrap();

    let rows = sqlx::query!(
        r#"SELECT key, run_at AS "run_at: jiff_sqlx::Timestamp" FROM scheduled_tasks WHERE application_id = 1 ORDER BY run_at"#
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let keys: Vec<_> = rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(keys, vec![format!("{prefix}mins_30"), format!("{prefix}now"),]);
    let last = rows.last().unwrap().run_at.to_jiff();
    assert!(last.duration_since(start).abs() < SignedDuration::from_millis(1));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn ensure_keeps_a_due_reminder(pool: PgPool) {
    let now = Timestamp::now();
    let row = post(now + SignedDuration::from_hours(2));
    let prefix = key_prefix(ThreadId::new(THREAD));
    ScheduledTasks::replace(
        &pool,
        ME.application_i64(),
        &prefix,
        &reminders(&row, now),
    )
    .await
    .unwrap();

    let key = format!("{prefix}mins_30");
    sqlx::query!(
        "UPDATE scheduled_tasks SET run_at = now() - interval '1 minute' WHERE application_id = 1 AND key = $1",
        key
    )
    .execute(&pool)
    .await
    .unwrap();

    ensure(&pool, ME, &row).await.unwrap();

    let run_at = sqlx::query_scalar!(
        r#"SELECT run_at AS "run_at: jiff_sqlx::Timestamp" FROM scheduled_tasks WHERE application_id = 1 AND key = $1"#,
        key
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(run_at.to_jiff() < Timestamp::now());
}

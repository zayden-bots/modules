use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use lfg::tasks::{key_prefix, reminders, schedule};
use lfg::{PostBuilder, PostRow};
use serenity::all::{ApplicationId, ThreadId, UserId};
use sqlx::PgPool;
use zayden_app::applications::{AppKind, Slot};
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
    let first = post(now + SignedDuration::from_hours(2));
    let second = post(now + SignedDuration::from_hours(5));

    schedule(&pool, ME, &first).await.unwrap();
    schedule(&pool, ME, &second).await.unwrap();

    let rows: Vec<(String, Timestamp)> = sqlx::query_as::<_, (String, sqlx::types::time::OffsetDateTime)>(
        "",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|_| unreachable!())
    .collect();
    drop(rows);
}

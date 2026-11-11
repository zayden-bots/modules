use jiff::{Span, Timestamp};
use jiff_sqlx::Timestamp as SqlxTimestamp;
use serenity::all::{ApplicationId, UserId};
use sqlx::PgPool;
use watch::party::PartyRow;
use watch::party::tasks::{self, key_prefix};
use zayden_app::applications::{AppKind, Slot};
use zayden_core::Identity;

const ME: Identity = Identity {
    application_id: ApplicationId::new(1),
    user_id: UserId::new(2),
    slot: Slot::Zayden,
    kind: AppKind::Default,
};

fn party(id: i64, starts_at: Timestamp) -> PartyRow {
    let ends_at = starts_at + Span::new().hours(2);
    PartyRow {
        id,
        guild_id: 1,
        channel_id: 2,
        message_id: None,
        event_id: None,
        host_id: 3,
        item_id: "item-1".to_owned(),
        item_name: "Arrival".to_owned(),
        item_type: "Movie".to_owned(),
        item_parent_path: "/media/movies".to_owned(),
        guests_enabled: true,
        library_name: None,
        library_item_id: None,
        starts_at: SqlxTimestamp::from(starts_at),
        ends_at: SqlxTimestamp::from(ends_at),
        cleanup_after: SqlxTimestamp::from(ends_at + Span::new().hours(2)),
        state: "scheduled".to_owned(),
        cleaned_at: None,
    }
}

#[test]
fn tasks_for_skips_past_instants() {
    let now = Timestamp::now();
    let row = party(7, now + Span::new().hours(2));

    let keys: Vec<String> =
        tasks::tasks_for(&row, now).into_iter().map(|t| t.key).collect();

    assert_eq!(keys, [
        "party:7:provision",
        "party:7:remind_1h",
        "party:7:remind_15m",
        "party:7:start",
        "party:7:cleanup",
    ]);
}

#[test]
fn rebuild_tasks_catches_up_provision_when_scheduled() {
    let now = Timestamp::now();
    let row = party(7, now + Span::new().minutes(10));

    let tasks = tasks::rebuild_tasks(&row, now);
    let provision: Vec<_> =
        tasks.iter().filter(|t| t.kind == tasks::PROVISION).collect();

    assert_eq!(provision.len(), 1);
    assert_eq!(provision[0].key, "party:7:provision");
    assert_eq!(provision[0].run_at, now);
    assert_eq!(provision[0].payload, serde_json::json!({ "party_id": 7 }));
    assert!(tasks.iter().any(|t| t.key == "party:7:cleanup"));

    let mut running = party(7, now + Span::new().minutes(10));
    running.state = "running".to_owned();
    assert!(
        tasks::rebuild_tasks(&running, now)
            .iter()
            .all(|t| t.kind != tasks::PROVISION)
    );
}

#[test]
fn rebuild_tasks_overdue_cleanup_only() {
    let now = Timestamp::now();
    let row = party(7, now - Span::new().hours(10));

    let tasks = tasks::rebuild_tasks(&row, now);

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, tasks::CLEANUP);
    assert_eq!(tasks[0].key, "party:7:cleanup");
    assert_eq!(tasks[0].run_at, now);
    assert_eq!(tasks[0].payload, serde_json::json!({ "party_id": 7 }));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn clear_removes_only_that_party(pool: PgPool) {
    let start = Timestamp::now() + Span::new().hours(48);
    tasks::schedule(&pool, ME, &party(1, start)).await.unwrap();
    tasks::schedule(&pool, ME, &party(2, start)).await.unwrap();

    tasks::clear(&pool, ME, 1).await.unwrap();

    let first = format!("{}%", key_prefix(1));
    let second = format!("{}%", key_prefix(2));
    let removed = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM scheduled_tasks WHERE key LIKE $1"#,
        first
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let kept = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM scheduled_tasks WHERE key LIKE $1"#,
        second
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(kept, 6);
}

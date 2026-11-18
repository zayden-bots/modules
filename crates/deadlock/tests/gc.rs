use deadlock::db::report_waits::{self, NewWait, Progress, WaitOutcome};
use deadlock::gc::{GcOutcome, run};
use jiff::{SignedDuration, Timestamp};
use sqlx::PgPool;

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("gc"))]
async fn expired_rows_are_pruned_and_recent_or_pinned_ones_survive(pool: PgPool) {
    let now: Timestamp = "2026-10-17T00:00:00Z".parse().unwrap();

    let outcome = run(&pool, now).await.unwrap();

    assert_eq!(outcome, GcOutcome {
        cache_rows: 1,
        rank_snapshots: 1,
        matches: 1,
        guild_posts: 1,
        briefings: 1,
        asset_snapshots: 2,
        report_waits: 0,
    });

    let keys = sqlx::query_scalar!("SELECT key FROM deadlock_api_cache")
        .fetch_all(&pool)
        .await
        .unwrap();
    let versions = sqlx::query_scalar!(
        "SELECT client_version FROM deadlock_asset_snapshots ORDER BY client_version"
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(keys, vec!["expired-two-days"]);
    assert_eq!(versions, vec![2, 4, 5, 6, 7, 8]);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn gc_prunes_week_old_finished_report_waits(pool: PgPool) {
    let now: Timestamp = "2026-10-17T00:00:00Z".parse().unwrap();
    let days = |n: i64| SignedDuration::from_hours(24 * n);

    for (match_id, ended_at, finished_at) in [
        (1_u64, now - days(9), Some(now - days(8))),
        (2, now - days(2), Some(now - days(1))),
        (3, now - days(30), None),
    ] {
        let wait = NewWait {
            account_id: 7,
            match_id,
            discord_id: 1,
            ended_at,
            reported_at: ended_at,
            expires_at: ended_at + days(1),
            next_attempt_at: ended_at,
        };
        assert!(report_waits::insert(&pool, &wait).await.unwrap());

        if let Some(finished_at) = finished_at {
            let progress = Progress {
                attempts: 0,
                next_attempt_at: ended_at,
                salts_at: None,
                metadata_at: None,
            };
            assert!(
                report_waits::finish(
                    &pool,
                    7,
                    match_id,
                    ended_at,
                    WaitOutcome::Ready,
                    finished_at,
                    &progress
                )
                .await
                .unwrap()
            );
        }
    }

    let outcome = run(&pool, now).await.unwrap();

    assert_eq!(outcome.report_waits, 1);
    assert!(report_waits::get(&pool, 7, 1).await.unwrap().is_none());
    assert!(report_waits::get(&pool, 7, 2).await.unwrap().is_some());
    assert!(report_waits::get(&pool, 7, 3).await.unwrap().is_some());
}

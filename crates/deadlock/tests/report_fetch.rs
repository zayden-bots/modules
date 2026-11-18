#![expect(clippy::unwrap_used, reason = "tests")]

mod common;

use std::time::Duration;

use common::{Mock, Reply, api, spawn_mock};
use deadlock::client::CacheKind;
use deadlock::db::matches::{self, SeenMatch};
use deadlock::db::report_waits::{self, NewWait, Progress, Wait, WaitOutcome};
use deadlock::db::{players, proxy_cache, user_settings};
use deadlock::report_fetch::{
    self,
    LEASE,
    MIN_GAP,
    STEAM_ACCOUNT_GAP,
    SteamLedger,
    WAIT_FOR,
    cache_key,
    due,
    first_attempt,
    metadata_canonical,
    metadata_players,
    parse_single_metadata,
    report_limiter,
    report_time_ok,
    salts_canonical,
    schedule_base,
    usable_salts,
};
use deadlock_api::{DeadlockClient, Limiter};
use jiff::{SignedDuration, Timestamp};
use sqlx::PgPool;
use sqlx::postgres::PgListener;
use zayden_app::events::DeadlockAppEvent;

const SALTS: &str = include_str!("fixtures/match_salts.json");
const METADATA: &str = include_str!("fixtures/match_metadata.json");
const ITEMS: &str = include_str!("fixtures/items.json");
const HEROES: &str = include_str!("fixtures/heroes.json");

const MATCH_ID: u64 = 69_192_735;
const ACCOUNT: u32 = 133_258_729;
const STEAM_ID: &str = "76561198093524457";
const DISCORD_ID: i64 = 1;

fn now() -> Timestamp {
    Timestamp::from_second(Timestamp::now().as_second()).unwrap()
}

fn not_found() -> Reply {
    Reply { status: 404, headers: Vec::new(), body: "{}".to_owned() }
}

fn client(mock: &Mock) -> DeadlockClient {
    DeadlockClient::builder()
        .base_url(&mock.base)
        .limiter(Limiter::with_configs([], Duration::ZERO))
        .build()
        .unwrap()
}

fn serve_match(mock: &Mock) {
    mock.serve("/v1/matches/69192735/salts", Reply::ok(SALTS));
    mock.serve("/v1/matches/69192735/metadata", Reply::ok(METADATA));
}

fn targets(mock: &Mock) -> Vec<String> {
    mock.requests.lock().map_or_else(
        |_| Vec::new(),
        |recorded| {
            recorded
                .iter()
                .filter_map(|head| head.split_whitespace().nth(1))
                .map(str::to_owned)
                .collect()
        },
    )
}

fn new_wait(account_id: u32, match_id: u64, now: Timestamp) -> NewWait {
    let reported_at = now - SignedDuration::from_mins(1);

    NewWait {
        account_id,
        match_id,
        discord_id: DISCORD_ID,
        ended_at: now - SignedDuration::from_mins(5),
        reported_at,
        expires_at: reported_at + WAIT_FOR,
        next_attempt_at: now - SignedDuration::from_secs(1),
    }
}

async fn add(pool: &PgPool, wait: &NewWait, attempts: u32) {
    assert!(report_waits::insert(pool, wait).await.unwrap());

    let progress = Progress {
        attempts,
        next_attempt_at: wait.next_attempt_at,
        salts_at: None,
        metadata_at: None,
    };
    report_waits::reschedule(
        pool,
        wait.account_id,
        wait.match_id,
        wait.next_attempt_at,
        &progress,
    )
    .await
    .unwrap();
}

async fn wait(
    pool: &PgPool,
    account_id: u32,
    now: Timestamp,
    attempts: u32,
) -> NewWait {
    let wait = new_wait(account_id, MATCH_ID, now);
    add(pool, &wait, attempts).await;

    wait
}

async fn row(pool: &PgPool, account_id: u32, match_id: u64) -> Wait {
    report_waits::get(pool, account_id, match_id).await.unwrap().unwrap()
}

async fn link_account(pool: &PgPool) {
    sqlx::query!(
        "INSERT INTO account_links (discord_id, provider, external_id) VALUES ($1, 'steam', $2)",
        DISCORD_ID,
        STEAM_ID
    )
    .execute(pool)
    .await
    .unwrap();
    players::sync(pool).await.unwrap();
    sqlx::query!("UPDATE deadlock_players SET seeded_at = '2026-01-01T00:00:00Z'")
        .execute(pool)
        .await
        .unwrap();
}

async fn cache_both(pool: &PgPool) {
    for (canonical, body) in [
        (salts_canonical(MATCH_ID), SALTS),
        (metadata_canonical(MATCH_ID), METADATA),
    ] {
        proxy_cache::put(
            pool,
            &cache_key(&canonical),
            CacheKind::Match,
            None,
            body,
            Duration::from_secs(60),
        )
        .await
        .unwrap();
    }
}

async fn seen_rows(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM deadlock_matches_seen"#)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[test]
fn schedule_follows_the_app() {
    let e: Timestamp = "2026-03-01T00:00:00Z".parse().unwrap();
    let now: Timestamp = "2026-03-10T00:00:00Z".parse().unwrap();
    let minutes = SignedDuration::from_mins;
    let days = |n: i64| SignedDuration::from_hours(24 * n);

    assert_eq!(due(e, 0), e + minutes(1));
    assert_eq!(due(e, 7), e + minutes(60));
    assert_eq!(due(e, 8), e + minutes(120));
    assert_eq!(due(e, u32::MAX), Timestamp::MAX);

    assert_eq!(first_attempt(now, now), now + minutes(1));
    assert_eq!(first_attempt(now - SignedDuration::from_hours(2), now), now);
    assert_eq!(first_attempt(e, now), due(schedule_base(e, now), 0));
    assert_eq!(first_attempt(now, now), due(schedule_base(now, now), 0));

    assert!(report_time_ok(now, now));
    assert!(report_time_ok(now - days(6), now));
    assert!(report_time_ok(now + minutes(5), now));
    assert!(!report_time_ok(now - days(8), now));
    assert!(!report_time_ok(now + minutes(11), now));
}

#[test]
fn single_metadata_decodes_into_the_bulk_model() {
    let meta = parse_single_metadata(METADATA).unwrap();

    assert_eq!(meta.match_id, MATCH_ID);
    assert_eq!(meta.start_time.as_second(), 1_774_071_946);
    assert_eq!(meta.winning_team, "Team0");
    assert_eq!(meta.game_mode, "Normal");

    let player = meta.players.first().unwrap();
    assert_eq!(player.team, "Team0");
    assert_eq!(player.player_level, 19);
    assert!(player.won(&meta.winning_team));

    assert_eq!(metadata_players(METADATA), Some(vec![ACCOUNT]));
    assert_eq!(metadata_players("[]"), None);
    assert_eq!(metadata_players("{}"), None);
    assert!(usable_salts(SALTS));
    assert!(!usable_salts(r#"{"metadata_url":null}"#));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn ready_when_deadlock_api_has_the_match(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 0).await;

    let (outcome, jobs) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.ready, 1);
    assert!(jobs.is_empty());

    let row = row(&pool, ACCOUNT, MATCH_ID).await;
    assert_eq!(row.outcome, Some(WaitOutcome::Ready));
    assert_eq!(row.finished_at, Some(now));
    assert_eq!(row.salts_at, Some(now));
    assert_eq!(row.metadata_at, Some(now));

    for (canonical, body) in [
        (salts_canonical(MATCH_ID), SALTS),
        (metadata_canonical(MATCH_ID), METADATA),
    ] {
        let cached =
            proxy_cache::get(&pool, &cache_key(&canonical)).await.unwrap().unwrap();
        assert_eq!(cached.body, body);
        assert!(cached.fresh);
    }

    let requested = targets(&mock);
    assert_eq!(requested.len(), 2);
    assert!(requested.iter().all(|target| target.ends_with("?disable_steam=true")));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn ready_pushes_match_ready(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 0).await;
    let mut listener = PgListener::connect_with(&pool).await.unwrap();
    listener.listen("deadlock_app").await.unwrap();

    report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
        .await
        .unwrap();

    let pushed = listener.recv().await.unwrap();
    assert_eq!(
        DeadlockAppEvent::decode(pushed.payload()),
        Some(DeadlockAppEvent::MatchReady {
            discord_id: DISCORD_ID,
            match_id: MATCH_ID
        })
    );
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn not_in_match_pushes_nothing(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, 7, now, 0).await;
    let mut listener = PgListener::connect_with(&pool).await.unwrap();
    listener.listen("deadlock_app").await.unwrap();

    report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
        .await
        .unwrap();

    let sentinel = DeadlockAppEvent::LinkChanged { discord_id: -1 };
    sentinel.notify(&pool).await.unwrap();
    let next = listener.recv().await.unwrap();
    assert_eq!(DeadlockAppEvent::decode(next.payload()), Some(sentinel));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn first_attempt_never_asks_steam(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    let added = wait(&pool, ACCOUNT, now, 0).await;

    report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
        .await
        .unwrap();

    assert_eq!(targets(&mock), vec![
        "/v1/matches/69192735/salts?disable_steam=true".to_owned(),
        "/v1/matches/69192735/metadata?disable_steam=true".to_owned(),
    ]);

    let row = row(&pool, ACCOUNT, MATCH_ID).await;
    let base = schedule_base(added.ended_at, added.reported_at);
    assert_eq!(row.attempts, 1);
    assert_eq!(row.next_attempt_at, due(base, 1).max(now + MIN_GAP));
    assert_eq!(row.finished_at, None);
}

fn serve_steam_only(mock: &Mock) {
    mock.serve_target("/v1/matches/69192735/salts?disable_steam=true", not_found());
    mock.serve_target("/v1/matches/69192735/salts", Reply::ok(SALTS));
}

fn steam_requests(mock: &Mock) -> usize {
    targets(mock).iter().filter(|target| !target.contains("disable_steam")).count()
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn steam_is_asked_on_attempts_one_to_three(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_steam_only(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 1).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.steam_calls, 1);
    assert_eq!(outcome.ready, 0);
    assert_eq!(targets(&mock), vec![
        "/v1/matches/69192735/salts?disable_steam=true".to_owned(),
        "/v1/matches/69192735/metadata?disable_steam=true".to_owned(),
        "/v1/matches/69192735/salts".to_owned(),
        "/v1/matches/69192735/metadata?disable_steam=true".to_owned(),
    ]);

    let cached = proxy_cache::get(&pool, &cache_key(&salts_canonical(MATCH_ID)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cached.body, SALTS);

    let row = row(&pool, ACCOUNT, MATCH_ID).await;
    assert_eq!(row.salts_at, Some(now));
    assert_eq!(row.attempts, 2);
    assert_eq!(row.finished_at, None);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn steam_is_not_asked_after_attempt_three(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_steam_only(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 4).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.steam_calls, 0);
    assert_eq!(targets(&mock), vec![
        "/v1/matches/69192735/salts?disable_steam=true".to_owned(),
        "/v1/matches/69192735/metadata?disable_steam=true".to_owned(),
    ]);

    let row = row(&pool, ACCOUNT, MATCH_ID).await;
    assert_eq!(row.attempts, 5);
    assert_eq!(row.finished_at, None);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn known_rows_skip_deadlock_api(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    cache_both(&pool).await;
    wait(&pool, ACCOUNT, now, 0).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.ready, 1);
    assert_eq!(targets(&mock), Vec::<String>::new());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn not_in_match_when_metadata_lacks_the_account(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, 7, now, 0).await;

    let (outcome, jobs) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.not_in_match, 1);
    assert!(jobs.is_empty());
    assert_eq!(row(&pool, 7, MATCH_ID).await.outcome, Some(WaitOutcome::NotInMatch));
    assert!(
        proxy_cache::get(&pool, &cache_key(&metadata_canonical(MATCH_ID)))
            .await
            .unwrap()
            .is_some()
    );
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn waits_expire_after_a_day(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    let mut expired = new_wait(ACCOUNT, MATCH_ID, now);
    expired.expires_at = now - SignedDuration::from_secs(1);
    add(&pool, &expired, 0).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.expired, 1);
    assert_eq!(outcome.claimed, 0);
    assert_eq!(
        row(&pool, ACCOUNT, MATCH_ID).await.outcome,
        Some(WaitOutcome::Expired)
    );
    assert_eq!(targets(&mock), Vec::<String>::new());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn opting_out_drops_the_wait(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 0).await;
    user_settings::set_match_fetch(&pool, DISCORD_ID, false, None).await.unwrap();

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.dropped, 1);
    assert!(report_waits::get(&pool, ACCOUNT, MATCH_ID).await.unwrap().is_none());
    assert_eq!(targets(&mock), Vec::<String>::new());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn busy_steam_bucket_holds_the_attempt(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let client = DeadlockClient::builder()
        .base_url(&mock.base)
        .limiter(report_limiter(false))
        .build()
        .unwrap();
    let now = now();
    let waits = [(1_u32, 1_001_u64), (2, 1_002), (3, 1_003)];
    for (account, id) in waits {
        add(&pool, &new_wait(account, id, now), 1).await;
    }

    let (outcome, _) =
        report_fetch::run(&client, &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.steam_calls, 2);
    assert_eq!(outcome.held, 1);

    let mut held = Vec::new();
    let mut advanced = 0;
    for (account, id) in waits {
        let row = row(&pool, account, id).await;
        if row.attempts == 2 {
            advanced += 1;
        } else {
            held.push(row);
        }
    }

    assert_eq!(advanced, 2);
    let [held] = held.as_slice() else { panic!("expected exactly one held wait") };
    assert_eq!(held.attempts, 1);
    assert!(held.next_attempt_at >= now + MIN_GAP);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn ready_feeds_one_summary(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    mock.serve("/v1/assets/items", Reply::ok(ITEMS));
    mock.serve("/v1/assets/heroes", Reply::ok(HEROES));
    let api = api(&pool, &mock.base).unwrap();
    link_account(&pool).await;
    let now = now();
    wait(&pool, ACCOUNT, now, 0).await;

    let (outcome, jobs) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.ready, 1);
    assert_eq!(outcome.summaries, 1);

    let [job] = jobs.as_slice() else { panic!("expected exactly one summary job") };
    assert_eq!(job.discord_id, DISCORD_ID);
    assert_eq!(job.summary.match_id, MATCH_ID);
    assert_eq!(job.summary.account_id, ACCOUNT);
    assert!(!job.recovered);
    assert_eq!(seen_rows(&pool).await, 1);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn match_poll_and_report_never_both_summarise(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    serve_match(&mock);
    mock.serve("/v1/assets/items", Reply::ok(ITEMS));
    mock.serve("/v1/assets/heroes", Reply::ok(HEROES));
    let api = api(&pool, &mock.base).unwrap();
    link_account(&pool).await;
    let now = now();
    wait(&pool, ACCOUNT, now, 0).await;
    let seen = SeenMatch {
        account_id: ACCOUNT,
        match_id: MATCH_ID,
        hero_id: 15,
        won: true,
        start_time: Timestamp::from_second(1_774_071_946).unwrap(),
    };
    assert!(matches::insert_if_new(&pool, &seen).await.unwrap());

    let (outcome, jobs) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.ready, 1);
    assert!(jobs.is_empty());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn claims_are_leased(pool: PgPool) {
    let now = now();
    for id in [1_u64, 2, 3] {
        add(&pool, &new_wait(ACCOUNT, id, now), 0).await;
    }
    let progress = Progress {
        attempts: 0,
        next_attempt_at: now,
        salts_at: None,
        metadata_at: None,
    };
    let leased = new_wait(ACCOUNT, 3, now).next_attempt_at;
    assert!(
        report_waits::finish(
            &pool,
            ACCOUNT,
            3,
            leased,
            WaitOutcome::Ready,
            now,
            &progress
        )
        .await
        .unwrap()
    );

    let first = report_waits::claim_due(&pool, now, now + LEASE, 10).await.unwrap();
    let second = report_waits::claim_due(&pool, now, now + LEASE, 10).await.unwrap();

    assert_eq!(first.len(), 2);
    assert_eq!(second, Vec::new());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn metadata_without_salts_is_enough(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    mock.serve("/v1/matches/69192735/metadata", Reply::ok(METADATA));
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    wait(&pool, ACCOUNT, now, 1).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.ready, 1);
    assert_eq!(outcome.steam_calls, 0);
    assert_eq!(steam_requests(&mock), 0);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn late_reports_never_ask_steam(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let now = now();
    let mut late = new_wait(ACCOUNT, MATCH_ID, now);
    late.ended_at = late.reported_at - SignedDuration::from_hours(5);
    add(&pool, &late, 1).await;

    let (outcome, _) =
        report_fetch::run(&client(&mock), &api, &pool, &SteamLedger::default(), now)
            .await
            .unwrap();

    assert_eq!(outcome.steam_calls, 0);
    assert_eq!(steam_requests(&mock), 0);

    let base = schedule_base(late.ended_at, late.reported_at);
    let row = row(&pool, ACCOUNT, MATCH_ID).await;
    assert_eq!(row.attempts, 2);
    assert_eq!(row.next_attempt_at, due(base, 2).max(now + MIN_GAP));
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn one_steam_pull_per_account_per_gap(pool: PgPool) {
    let mock = spawn_mock(not_found()).await.unwrap();
    let api = api(&pool, &mock.base).unwrap();
    let client = client(&mock);
    let ledger = SteamLedger::default();
    let now = now();
    for id in [1_001_u64, 1_002] {
        add(&pool, &new_wait(ACCOUNT, id, now), 1).await;
    }

    report_fetch::run(&client, &api, &pool, &ledger, now).await.unwrap();
    assert_eq!(steam_requests(&mock), 1);

    let later = now + STEAM_ACCOUNT_GAP;
    report_fetch::run(&client, &api, &pool, &ledger, later).await.unwrap();
    assert_eq!(steam_requests(&mock), 2);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn stale_lease_writes_nothing(pool: PgPool) {
    let now = now();
    add(&pool, &new_wait(ACCOUNT, MATCH_ID, now), 0).await;
    let claimed =
        report_waits::claim_due(&pool, now, now + LEASE, 10).await.unwrap();
    let [claimed] = claimed.as_slice() else { panic!("expected one claimed wait") };
    let stale = claimed.next_attempt_at + SignedDuration::from_secs(1);
    let progress = Progress {
        attempts: 5,
        next_attempt_at: now + MIN_GAP,
        salts_at: None,
        metadata_at: None,
    };

    report_waits::reschedule(&pool, ACCOUNT, MATCH_ID, stale, &progress)
        .await
        .unwrap();
    let finished = report_waits::finish(
        &pool,
        ACCOUNT,
        MATCH_ID,
        stale,
        WaitOutcome::Ready,
        now,
        &progress,
    )
    .await
    .unwrap();

    assert!(!finished);
    let unchanged = row(&pool, ACCOUNT, MATCH_ID).await;
    assert_eq!(unchanged.attempts, 0);
    assert_eq!(unchanged.finished_at, None);

    report_waits::reschedule(
        &pool,
        ACCOUNT,
        MATCH_ID,
        claimed.next_attempt_at,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(row(&pool, ACCOUNT, MATCH_ID).await.attempts, 5);

    let finished = report_waits::finish(
        &pool,
        ACCOUNT,
        MATCH_ID,
        progress.next_attempt_at,
        WaitOutcome::Ready,
        now,
        &progress,
    )
    .await
    .unwrap();
    assert!(finished);
}

#[test]
fn single_metadata_accepts_nulls() {
    let mut value: serde_json::Value = serde_json::from_str(METADATA).unwrap();
    for (pointer, keys) in [
        ("/match_info", vec!["game_mode"]),
        ("/match_info/players/0", vec!["level", "assigned_lane", "kills", "items"]),
    ] {
        let object = value.pointer_mut(pointer).unwrap().as_object_mut().unwrap();
        for key in keys {
            object.insert(key.to_owned(), serde_json::Value::Null);
        }
    }

    let meta = parse_single_metadata(&value.to_string()).unwrap();

    assert_eq!(meta.game_mode, "0");
    assert_eq!(meta.players.first().unwrap().player_level, 0);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn insert_is_idempotent(pool: PgPool) {
    let now = now();
    let new = new_wait(ACCOUNT, MATCH_ID, now);

    assert!(report_waits::insert(&pool, &new).await.unwrap());
    assert!(!report_waits::insert(&pool, &new).await.unwrap());

    assert_eq!(row(&pool, ACCOUNT, MATCH_ID).await, Wait {
        account_id: new.account_id,
        match_id: new.match_id,
        discord_id: new.discord_id,
        ended_at: new.ended_at,
        reported_at: new.reported_at,
        expires_at: new.expires_at,
        next_attempt_at: new.next_attempt_at,
        attempts: 0,
        salts_at: None,
        metadata_at: None,
        finished_at: None,
        outcome: None,
    });
}

#![expect(clippy::unwrap_used, reason = "tests")]

mod common;

use std::time::Duration;

use common::{Reply, Requests, api, spawn_mock};
use deadlock::client::CacheKind;
use deadlock::db::proxy_cache;
use deadlock::proxy::{Lookup, ProxyBody, ProxyClass, client_version};
use deadlock::{DeadlockProxy, ProxyError};
use deadlock_api::{DeadlockClient, Limiter};
use sqlx::PgPool;

const HEROES: &str = include_str!("fixtures/heroes.json");
const ITEM_FLOW: &str = include_str!("fixtures/item_flow.json");
const BUILDS: &str = include_str!("fixtures/builds.json");
const MATCH_METADATA: &str = include_str!("fixtures/match_metadata.json");
const MATCH_SALTS: &str = include_str!("fixtures/match_salts.json");

const HEROES_KEY: &str = "proxy:/v1/assets/heroes?client_version=6123";
const FLOW_KEY: &str = "proxy:/v1/analytics/item-flow-stats?hero_ids=1";

fn count(requests: &Requests) -> usize {
    requests.lock().map_or(0, |recorded| recorded.len())
}

fn proxy(pool: &PgPool, base: &str) -> deadlock_api::Result<DeadlockProxy> {
    let client = DeadlockClient::builder()
        .base_url(base)
        .limiter(Limiter::with_configs([], Duration::ZERO))
        .build()?;

    Ok(DeadlockProxy::from_client(client, pool.clone()))
}

async fn get_through(
    proxy: &DeadlockProxy,
    canonical: &str,
) -> Result<ProxyBody, ProxyError> {
    match proxy.lookup(canonical).await? {
        Lookup::Fresh(body) => Ok(body),
        Lookup::Miss(miss) => proxy.fetch(miss).await,
    }
}

struct Stored {
    kind: String,
    client_version: Option<i32>,
    ttl_secs: f64,
}

async fn stored(pool: &PgPool, key: &str) -> Stored {
    let row = sqlx::query!(
        r#"SELECT kind, client_version,
            EXTRACT(EPOCH FROM expires_at - fetched_at)::float8 AS "ttl_secs!"
        FROM deadlock_api_cache WHERE key = $1"#,
        key
    )
    .fetch_one(pool)
    .await
    .unwrap();

    Stored {
        kind: row.kind,
        client_version: row.client_version,
        ttl_secs: row.ttl_secs,
    }
}

fn assert_stored(
    stored: &Stored,
    kind: &str,
    client_version: Option<i32>,
    ttl_secs: f64,
) {
    assert_eq!(stored.kind, kind);
    assert_eq!(stored.client_version, client_version);
    assert!(
        (stored.ttl_secs - ttl_secs).abs() <= 1.0,
        "ttl {} vs {ttl_secs}",
        stored.ttl_secs
    );
}

#[test]
fn classes_follow_the_path() {
    let class = ProxyClass::of;

    assert_eq!(class("/v1/assets/heroes?client_version=1"), Some(ProxyClass::Asset));
    assert_eq!(
        class("/v1/assets/client-versions"),
        Some(ProxyClass::ClientVersions)
    );
    assert_eq!(
        class("/v1/analytics/item-flow-stats?hero_ids=1"),
        Some(ProxyClass::Analytics)
    );
    assert_eq!(class("/v1/builds?hero_id=1"), Some(ProxyClass::Build));
    assert_eq!(
        class("/v1/matches/1/salts?disable_steam=true"),
        Some(ProxyClass::Match)
    );
    assert_eq!(class("/v1/matches/1/metadata"), None);
    assert_eq!(class("/v1/matches/1/salts?disable_steam=false"), None);
    assert_eq!(class("/v1/players/1/match-history"), None);
    assert_eq!(class("/v1/buildsx"), None);
}

#[test]
fn client_version_reads_the_query() {
    assert_eq!(client_version("/v1/assets/heroes?client_version=6123"), Some(6123));
    assert_eq!(client_version("/v1/assets/heroes"), None);
    assert_eq!(client_version("/v1/assets/heroes?client_version=4294967295"), None);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn a_miss_fetches_and_stores_the_body_verbatim(pool: PgPool) {
    let mock = spawn_mock(Reply::ok(HEROES)).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();

    let body =
        get_through(&proxy, "/v1/assets/heroes?client_version=6123").await.unwrap();

    assert_eq!(body.body, HEROES);
    assert!(!body.stale);

    let head = mock.requests.lock().unwrap().first().cloned().unwrap();
    assert!(
        head.starts_with("GET /v1/assets/heroes?client_version=6123 HTTP/1.1"),
        "{head}"
    );

    assert_stored(&stored(&pool, HEROES_KEY).await, "asset", Some(6123), 86400.0);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn a_fresh_row_is_served_without_upstream(pool: PgPool) {
    let mock = spawn_mock(Reply::ok(HEROES)).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/assets/heroes?client_version=6123";

    let first = get_through(&proxy, canonical).await.unwrap();
    let again = proxy.lookup(canonical).await.unwrap();

    let Lookup::Fresh(again) = again else {
        panic!("expected a fresh row");
    };

    assert_eq!(again.body, first.body);
    assert_eq!(
        again.fetched_at,
        proxy_cache::get(&pool, HEROES_KEY).await.unwrap().unwrap().fetched_at
    );
    assert!(!again.stale);
    assert_eq!(count(&mock.requests), 1);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn ttl_and_kind_by_class(pool: PgPool) {
    let mock = spawn_mock(Reply::ok("[]")).await.unwrap();
    mock.serve("/v1/assets/client-versions", Reply::ok("[6123]"));
    mock.serve("/v1/analytics/item-flow-stats", Reply::ok(ITEM_FLOW));
    mock.serve("/v1/builds", Reply::ok(BUILDS));
    mock.serve("/v1/matches/69192735/metadata", Reply::ok(MATCH_METADATA));
    mock.serve("/v1/matches/69192735/salts", Reply::ok(MATCH_SALTS));
    let proxy = proxy(&pool, &mock.base).unwrap();

    let cases = [
        ("/v1/assets/client-versions", "asset", 300.0),
        ("/v1/analytics/item-flow-stats?hero_ids=1", "analytics", 21600.0),
        ("/v1/builds?hero_id=1", "build", 21600.0),
        ("/v1/matches/69192735/metadata?disable_steam=true", "match", 604_800.0),
        ("/v1/matches/69192735/salts?disable_steam=true", "match", 604_800.0),
    ];

    for (canonical, kind, ttl) in cases {
        get_through(&proxy, canonical).await.unwrap();
        let key = format!("proxy:{canonical}");
        assert_stored(&stored(&pool, &key).await, kind, None, ttl);
    }
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn not_found_is_never_cached(pool: PgPool) {
    let mock =
        spawn_mock(Reply { status: 404, headers: Vec::new(), body: String::new() })
            .await
            .unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/matches/1/metadata?disable_steam=true";

    let first = get_through(&proxy, canonical).await.unwrap_err();
    let second = get_through(&proxy, canonical).await.unwrap_err();

    assert!(matches!(first, ProxyError::NotFound), "{first:?}");
    assert!(matches!(second, ProxyError::NotFound), "{second:?}");
    assert!(
        proxy_cache::get(&pool, &format!("proxy:{canonical}"))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(count(&mock.requests), 2);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn client_errors_are_rejected(pool: PgPool) {
    let mock =
        spawn_mock(Reply { status: 400, headers: Vec::new(), body: String::new() })
            .await
            .unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/builds?hero_id=1";

    let err = get_through(&proxy, canonical).await.unwrap_err();

    assert!(matches!(err, ProxyError::Rejected { status: 400 }), "{err:?}");
    assert!(
        proxy_cache::get(&pool, &format!("proxy:{canonical}"))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn a_stale_row_is_served_when_upstream_fails(pool: PgPool) {
    let mock =
        spawn_mock(Reply { status: 503, headers: Vec::new(), body: String::new() })
            .await
            .unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/analytics/item-flow-stats?hero_ids=1";

    let err = get_through(&proxy, canonical).await.unwrap_err();
    assert!(matches!(err, ProxyError::Upstream(_)), "{err:?}");

    let fetched_at = proxy_cache::put(
        &pool,
        FLOW_KEY,
        CacheKind::Analytics,
        None,
        "[1]",
        Duration::ZERO,
    )
    .await
    .unwrap();

    let body = get_through(&proxy, canonical).await.unwrap();

    assert_eq!(body.body, "[1]");
    assert!(body.stale);
    assert_eq!(body.fetched_at, fetched_at);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn a_stale_row_is_not_served_for_not_found(pool: PgPool) {
    let mock =
        spawn_mock(Reply { status: 404, headers: Vec::new(), body: String::new() })
            .await
            .unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();

    proxy_cache::put(
        &pool,
        "proxy:/v1/matches/1/metadata?disable_steam=true",
        CacheKind::Match,
        None,
        "{}",
        Duration::ZERO,
    )
    .await
    .unwrap();

    let err = get_through(&proxy, "/v1/matches/1/metadata?disable_steam=true")
        .await
        .unwrap_err();

    assert!(matches!(err, ProxyError::NotFound), "{err:?}");
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn a_non_json_body_is_not_cached(pool: PgPool) {
    let mock = spawn_mock(Reply::ok("<html>")).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/builds?hero_id=1";

    let err = get_through(&proxy, canonical).await.unwrap_err();

    assert!(matches!(err, ProxyError::InvalidBody(_)), "{err:?}");
    assert!(
        proxy_cache::get(&pool, &format!("proxy:{canonical}"))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn unproxied_paths_are_refused(pool: PgPool) {
    let mock = spawn_mock(Reply::ok("[]")).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();

    let err = proxy.lookup("/v1/players/1/match-history").await.unwrap_err();

    assert!(matches!(err, ProxyError::NotProxied(_)), "{err:?}");
    assert_eq!(count(&mock.requests), 0);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn concurrent_misses_share_one_fetch(pool: PgPool) {
    let mock = spawn_mock(Reply::ok(HEROES)).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/assets/heroes?client_version=6123";

    let (Lookup::Miss(a), Lookup::Miss(b)) = (
        proxy.lookup(canonical).await.unwrap(),
        proxy.lookup(canonical).await.unwrap(),
    ) else {
        panic!("expected two misses");
    };

    let (a, b) = tokio::join!(proxy.fetch(a), proxy.fetch(b));

    assert_eq!(a.unwrap().body, HEROES);
    assert_eq!(b.unwrap().body, HEROES);
    assert_eq!(count(&mock.requests), 1);
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn an_oversized_body_is_not_cached(pool: PgPool) {
    let body = format!("\"{}\"", "a".repeat(16 * 1024 * 1024 - 1));
    let mock = spawn_mock(Reply::ok(&body)).await.unwrap();
    let proxy = proxy(&pool, &mock.base).unwrap();
    let canonical = "/v1/builds?hero_id=1";

    let err = get_through(&proxy, canonical).await.unwrap_err();

    assert!(matches!(err, ProxyError::TooLarge { .. }), "{err:?}");
    assert!(
        proxy_cache::get(&pool, &format!("proxy:{canonical}"))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn invalidation_keeps_match_and_versioned_rows(pool: PgPool) {
    let mock = spawn_mock(Reply::ok("[]")).await.unwrap();
    let hour = Duration::from_hours(1);
    let rows = [
        ("proxy:/v1/matches/1/metadata?disable_steam=true", CacheKind::Match, None),
        ("proxy:/v1/assets/heroes?client_version=1", CacheKind::Asset, Some(1)),
        ("proxy:/v1/assets/heroes", CacheKind::Asset, None),
        (FLOW_KEY, CacheKind::Analytics, None),
    ];

    for (key, kind, version) in rows {
        proxy_cache::put(&pool, key, kind, version, "[]", hour).await.unwrap();
    }

    api(&pool, &mock.base).unwrap().invalidate_unversioned().await.unwrap();

    for (key, ..) in rows.iter().take(2) {
        assert!(proxy_cache::get(&pool, key).await.unwrap().is_some(), "{key}");
    }
    for (key, ..) in rows.iter().skip(2) {
        assert!(proxy_cache::get(&pool, key).await.unwrap().is_none(), "{key}");
    }
}

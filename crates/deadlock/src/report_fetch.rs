use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::sync::Mutex;
use std::time::Duration;

use deadlock_api::model::{MatchMetadata, MatchPlayer, PlayerItem, StatSnapshot};
use deadlock_api::{
    Bucket,
    BucketConfig,
    DeadlockApiError,
    DeadlockClient,
    Limiter,
    Priority,
};
use jiff::{SignedDuration, Timestamp};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sqlx::PgPool;
use tracing::{debug, info, warn};
use zayden_app::events::DeadlockAppEvent;

use crate::client::{CacheKind, DeadlockApi};
use crate::db::matches::{self, SeenMatch};
use crate::db::report_waits::{self, Progress, Wait, WaitOutcome};
use crate::db::{players, proxy_cache, user_settings};
use crate::error::DeadlockError;
use crate::items::ItemIndex;
use crate::match_poll::{self, SummaryJob};
use crate::proxy::{self, ProxyClass};
use crate::summary;

pub const SCHEDULE: [SignedDuration; 8] = [
    SignedDuration::from_mins(1),
    SignedDuration::from_mins(3),
    SignedDuration::from_mins(6),
    SignedDuration::from_mins(10),
    SignedDuration::from_mins(15),
    SignedDuration::from_mins(25),
    SignedDuration::from_mins(40),
    SignedDuration::from_mins(60),
];
pub const RETRY_EVERY: SignedDuration = SignedDuration::from_hours(1);
pub const WAIT_FOR: SignedDuration = SignedDuration::from_hours(24);
pub const MAX_REPORT_AGE: SignedDuration = SignedDuration::from_hours(24 * 7);
pub const MAX_CLOCK_SKEW: SignedDuration = SignedDuration::from_mins(10);
pub const STEAM_ATTEMPTS: RangeInclusive<u32> = 1..=3;
pub const MIN_GAP: SignedDuration = SignedDuration::from_mins(1);
pub const MAX_HOLD: SignedDuration = SignedDuration::from_mins(15);
pub const LEASE: SignedDuration = SignedDuration::from_mins(10);
pub const STEAM_MAX_AGE: SignedDuration = SignedDuration::from_hours(3);
pub const STEAM_ACCOUNT_GAP: SignedDuration = SignedDuration::from_mins(20);
pub const BATCH: i64 = 10;
pub const DB_BUCKET: Bucket = Bucket::Assets;
pub const STEAM_BUCKET: Bucket = Bucket::SingleMetadata;

#[must_use]
pub fn due(ended_at: Timestamp, attempt: u32) -> Timestamp {
    let offset = usize::try_from(attempt)
        .ok()
        .and_then(|n| SCHEDULE.get(n))
        .copied()
        .or_else(|| {
            i32::try_from(attempt.saturating_sub(7))
                .ok()
                .and_then(|extra| RETRY_EVERY.checked_mul(extra))
                .and_then(|later| SignedDuration::from_mins(60).checked_add(later))
        });

    offset
        .and_then(|offset| ended_at.checked_add(offset).ok())
        .unwrap_or(Timestamp::MAX)
}

#[must_use]
pub fn schedule_base(ended_at: Timestamp, reported_at: Timestamp) -> Timestamp {
    ended_at.max(reported_at.checked_sub(MIN_GAP).unwrap_or(Timestamp::MIN))
}

#[derive(Debug, Default)]
pub struct SteamLedger {
    taken: Mutex<HashMap<u32, Timestamp>>,
}

impl SteamLedger {
    /// Takes the account's Steam slot when its last one is at least
    /// `STEAM_ACCOUNT_GAP` old.
    pub fn try_take(&self, account_id: u32, now: Timestamp) -> bool {
        let Ok(mut taken) = self.taken.lock() else {
            return false;
        };

        taken.retain(|_, at| now.duration_since(*at) < STEAM_ACCOUNT_GAP);
        if taken.contains_key(&account_id) {
            return false;
        }

        taken.insert(account_id, now);
        true
    }
}

fn steam_allowed(wait: &Wait, now: Timestamp, ledger: &SteamLedger) -> bool {
    STEAM_ATTEMPTS.contains(&wait.attempts)
        && wait.reported_at.duration_since(wait.ended_at) <= STEAM_MAX_AGE
        && ledger.try_take(wait.account_id, now)
}

#[must_use]
pub fn first_attempt(ended_at: Timestamp, now: Timestamp) -> Timestamp {
    due(ended_at, 0).max(now)
}

#[must_use]
pub fn report_time_ok(ended_at: Timestamp, now: Timestamp) -> bool {
    now.checked_sub(MAX_REPORT_AGE).unwrap_or(Timestamp::MIN) <= ended_at
        && ended_at <= now.checked_add(MAX_CLOCK_SKEW).unwrap_or(Timestamp::MAX)
}

#[must_use]
pub fn salts_canonical(match_id: u64) -> String {
    format!("/v1/matches/{match_id}/salts?disable_steam=true")
}

#[must_use]
pub fn metadata_canonical(match_id: u64) -> String {
    format!("/v1/matches/{match_id}/metadata?disable_steam=true")
}

#[must_use]
pub fn cache_key(canonical: &str) -> String {
    format!("{}{canonical}", proxy::KEY_PREFIX)
}

// Own limiter: DB_BUCKET is DB/S3 match reads (100/s salts, 100 per 10 s metadata
// per IP), STEAM_BUCKET is Steam salt pulls (3/h per IP, 300/h per key).
#[must_use]
pub fn report_limiter(has_api_key: bool) -> Limiter {
    let steam = BucketConfig {
        capacity: if has_api_key { 200 } else { 2 },
        per: Duration::from_hours(1),
        reserved: 0,
    };

    Limiter::with_configs(
        [
            (DB_BUCKET, BucketConfig {
                capacity: 30,
                per: Duration::from_secs(10),
                reserved: 0,
            }),
            (STEAM_BUCKET, steam),
        ],
        Duration::ZERO,
    )
}

pub fn report_client(
    http: reqwest::Client,
    api_key: Option<&SecretString>,
) -> crate::Result<DeadlockClient> {
    let mut builder = DeadlockClient::builder()
        .http(http)
        .limiter(report_limiter(api_key.is_some()));

    if let Some(key) = api_key {
        builder = builder.api_key(key.expose_secret());
    }

    Ok(builder.build()?)
}

#[derive(Deserialize)]
struct SingleMetadata {
    match_info: SingleMatchInfo,
}

#[derive(Deserialize)]
struct SingleMatchInfo {
    match_id: u64,
    start_time: i64,
    winning_team: i64,
    duration_s: u32,
    #[serde(default, deserialize_with = "null_default")]
    game_mode: i64,
    #[serde(default, deserialize_with = "null_default")]
    match_mode: i64,
    #[serde(default, deserialize_with = "null_default")]
    players: Vec<SinglePlayer>,
}

#[derive(Deserialize)]
struct SinglePlayer {
    account_id: u32,
    hero_id: u32,
    team: i64,
    #[serde(default, deserialize_with = "null_default")]
    kills: u32,
    #[serde(default, deserialize_with = "null_default")]
    deaths: u32,
    #[serde(default, deserialize_with = "null_default")]
    assists: u32,
    #[serde(default, deserialize_with = "null_default")]
    net_worth: u32,
    #[serde(default, deserialize_with = "null_default")]
    last_hits: u32,
    #[serde(default, deserialize_with = "null_default")]
    denies: u32,
    #[serde(default, deserialize_with = "null_default")]
    level: u32,
    #[serde(default, deserialize_with = "null_default")]
    assigned_lane: u8,
    #[serde(default, deserialize_with = "null_default")]
    items: Vec<PlayerItem>,
    #[serde(default, deserialize_with = "null_default")]
    stats: Vec<StatSnapshot>,
}

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl From<SinglePlayer> for MatchPlayer {
    fn from(player: SinglePlayer) -> Self {
        Self {
            account_id: player.account_id,
            hero_id: player.hero_id,
            team: format!("Team{}", player.team),
            kills: player.kills,
            deaths: player.deaths,
            assists: player.assists,
            net_worth: player.net_worth,
            last_hits: player.last_hits,
            denies: player.denies,
            player_level: player.level,
            assigned_lane: player.assigned_lane,
            player_match_outcome: None,
            items: player.items,
            stats: player.stats,
        }
    }
}

pub fn parse_single_metadata(
    body: &str,
) -> Result<MatchMetadata, serde_json::Error> {
    let SingleMetadata { match_info: info } = serde_json::from_str(body)?;
    let start_time =
        Timestamp::from_second(info.start_time).map_err(serde::de::Error::custom)?;

    Ok(MatchMetadata {
        match_id: info.match_id,
        start_time,
        winning_team: format!("Team{}", info.winning_team),
        duration_s: info.duration_s,
        game_mode: if info.game_mode == 1 {
            "Normal".to_owned()
        } else {
            info.game_mode.to_string()
        },
        match_mode: info.match_mode.to_string(),
        players: info.players.into_iter().map(MatchPlayer::from).collect(),
    })
}

#[must_use]
pub fn usable_salts(body: &str) -> bool {
    body.len() <= proxy::MAX_BODY_BYTES
        && serde_json::from_str::<Value>(body).is_ok_and(|value| {
            value
                .as_object()
                .and_then(|object| object.get("metadata_url"))
                .and_then(Value::as_str)
                .is_some_and(|url| !url.is_empty())
        })
}

#[must_use]
pub fn metadata_players(body: &str) -> Option<Vec<u32>> {
    if body.len() > proxy::MAX_BODY_BYTES {
        return None;
    }

    let value: Value = serde_json::from_str(body).ok()?;
    let info = value.as_object()?.get("match_info")?.as_object()?;
    let accounts = info.get("players").and_then(Value::as_array).map_or_else(
        Vec::new,
        |players| {
            players
                .iter()
                .filter_map(|player| {
                    let id = player.get("account_id")?.as_u64()?;
                    u32::try_from(id).ok()
                })
                .collect()
        },
    );

    Some(accounts)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReportOutcome {
    pub expired: u64,
    pub claimed: usize,
    pub ready: usize,
    pub not_in_match: usize,
    pub dropped: usize,
    pub held: usize,
    pub steam_calls: usize,
    pub summaries: usize,
}

enum Pulled {
    Body(String),
    Failed(String),
    Busy(Duration),
}

enum Reach<T> {
    Got(T),
    NotYet,
    Hold(Duration),
}

async fn pull(client: &DeadlockClient, path: &str, bucket: Bucket) -> Pulled {
    match client.get_raw(path, bucket, Priority::Background).await {
        Ok(body) => Pulled::Body(body),
        Err(DeadlockApiError::Busy { retry_after }) => Pulled::Busy(retry_after),
        Err(e) => Pulled::Failed(e.to_string()),
    }
}

async fn cached(
    pool: &PgPool,
    key: &str,
    usable: fn(&str) -> bool,
) -> sqlx::Result<Option<String>> {
    Ok(proxy_cache::get(pool, key)
        .await?
        .filter(|row| row.fresh && usable(&row.body))
        .map(|row| row.body))
}

fn usable_metadata(body: &str) -> bool {
    metadata_players(body).is_some()
}

async fn store(pool: &PgPool, key: &str, body: &str) -> sqlx::Result<()> {
    proxy_cache::put(
        pool,
        key,
        CacheKind::Match,
        None,
        body,
        ProxyClass::Match.ttl(),
    )
    .await?;

    Ok(())
}

async fn salts(
    client: &DeadlockClient,
    pool: &PgPool,
    wait: &Wait,
    now: Timestamp,
    progress: &mut Progress,
    steam: Option<&mut ReportOutcome>,
) -> crate::Result<Reach<()>> {
    let id = wait.match_id;
    let key = cache_key(&salts_canonical(id));

    let path = if steam.is_some() {
        format!("/v1/matches/{id}/salts")
    } else {
        if cached(pool, &key, usable_salts).await?.is_some() {
            progress.salts_at = Some(now);
            return Ok(Reach::Got(()));
        }
        salts_canonical(id)
    };
    let bucket = if steam.is_some() { STEAM_BUCKET } else { DB_BUCKET };

    let pulled = pull(client, &path, bucket).await;
    if let Pulled::Busy(retry_after) = pulled {
        return Ok(Reach::Hold(retry_after));
    }
    if let Some(outcome) = steam {
        outcome.steam_calls += 1;
    }

    match pulled {
        Pulled::Body(body) if usable_salts(&body) => {
            store(pool, &key, &body).await?;
            progress.salts_at = Some(now);
            return Ok(Reach::Got(()));
        },
        Pulled::Body(_) => {
            debug!(match_id = id, "deadlock: report salts body unusable");
        },
        Pulled::Failed(reason) => {
            debug!(match_id = id, reason, "deadlock: report salts not available");
        },
        Pulled::Busy(_) => {},
    }

    Ok(Reach::NotYet)
}

async fn metadata(
    client: &DeadlockClient,
    pool: &PgPool,
    wait: &Wait,
    now: Timestamp,
    progress: &mut Progress,
) -> crate::Result<Reach<String>> {
    let id = wait.match_id;
    let canonical = metadata_canonical(id);
    let key = cache_key(&canonical);

    if let Some(body) = cached(pool, &key, usable_metadata).await? {
        progress.metadata_at = Some(now);
        return Ok(Reach::Got(body));
    }

    match pull(client, &canonical, DB_BUCKET).await {
        Pulled::Busy(retry_after) => Ok(Reach::Hold(retry_after)),
        Pulled::Body(body) if usable_metadata(&body) => {
            store(pool, &key, &body).await?;
            progress.metadata_at = Some(now);
            Ok(Reach::Got(body))
        },
        Pulled::Body(_) => {
            debug!(match_id = id, "deadlock: report metadata body unusable");
            Ok(Reach::NotYet)
        },
        Pulled::Failed(reason) => {
            debug!(match_id = id, reason, "deadlock: report metadata not available");
            Ok(Reach::NotYet)
        },
    }
}

async fn summary_job(
    api: &DeadlockApi,
    pool: &PgPool,
    wait: &Wait,
    body: &str,
) -> crate::Result<Option<SummaryJob>> {
    let account_id = wait.account_id;
    let match_id = wait.match_id;

    let tracked = match players::get(pool, account_id).await? {
        Some(tracked) => tracked,
        None => {
            players::sync(pool).await?;
            let Some(tracked) = players::get(pool, account_id).await? else {
                debug!(
                    match_id,
                    account_id,
                    "deadlock: report summary skipped, player not tracked"
                );
                return Ok(None);
            };
            tracked
        },
    };

    let meta = match parse_single_metadata(body) {
        Ok(meta) => meta,
        Err(e) => {
            warn!(error = %e, match_id, account_id, "deadlock: report metadata did not decode");
            return Ok(None);
        },
    };

    if meta.game_mode != "Normal" {
        debug!(
            match_id,
            account_id, "deadlock: report summary skipped, not a normal game"
        );
        return Ok(None);
    }

    let Some(player) = meta.players.iter().find(|p| p.account_id == account_id)
    else {
        debug!(
            match_id,
            account_id, "deadlock: report summary skipped, player missing"
        );
        return Ok(None);
    };

    if meta.start_time < tracked.seeded_at {
        debug!(
            match_id,
            account_id, "deadlock: report summary skipped, match predates the link"
        );
        return Ok(None);
    }

    let items = match api.items(None, Priority::Background).await {
        Ok(items) => items,
        Err(DeadlockError::Api(e @ DeadlockApiError::Busy { .. })) => {
            debug!(error = %e, match_id, account_id, "deadlock: report summary deferred, deadlock-api is busy");
            return Ok(None);
        },
        Err(e) => {
            warn!(error = ?e, match_id, account_id, "deadlock: report summary skipped, items unavailable");
            return Ok(None);
        },
    };

    let seen = SeenMatch {
        account_id,
        match_id,
        hero_id: player.hero_id,
        won: player.won(&meta.winning_team),
        start_time: meta.start_time,
    };
    if !matches::insert_if_new(pool, &seen).await? {
        debug!(
            match_id,
            account_id, "deadlock: report summary skipped, already seen"
        );
        return Ok(None);
    }

    if let Err(e) = match_poll::refresh_streak(pool, &tracked).await {
        warn!(error = %e, match_id, account_id, "deadlock: report streak refresh failed");
    }
    let averages = match_poll::hero_averages(api, player.hero_id).await;

    Ok(Some(SummaryJob {
        discord_id: tracked.discord_id,
        recovered: false,
        summary: summary::summarize(
            &meta,
            player,
            &averages,
            &ItemIndex::new(&items.value),
        ),
    }))
}

fn capped(at: Timestamp, expires_at: Timestamp) -> Timestamp {
    at.min(expires_at)
}

async fn advance(
    client: &DeadlockClient,
    pool: &PgPool,
    wait: &Wait,
    now: Timestamp,
    ledger: &SteamLedger,
    progress: &mut Progress,
    outcome: &mut ReportOutcome,
) -> crate::Result<Reach<String>> {
    if progress.salts_at.is_none()
        && let Reach::Hold(retry_after) =
            salts(client, pool, wait, now, progress, None).await?
    {
        return Ok(Reach::Hold(retry_after));
    }

    let found = metadata(client, pool, wait, now, progress).await?;
    if !matches!(found, Reach::NotYet) {
        return Ok(found);
    }

    if progress.salts_at.is_some() || !steam_allowed(wait, now, ledger) {
        return Ok(Reach::NotYet);
    }

    match salts(client, pool, wait, now, progress, Some(outcome)).await? {
        Reach::Got(()) => metadata(client, pool, wait, now, progress).await,
        Reach::NotYet => Ok(Reach::NotYet),
        Reach::Hold(retry_after) => Ok(Reach::Hold(retry_after)),
    }
}

async fn step(
    client: &DeadlockClient,
    api: &DeadlockApi,
    pool: &PgPool,
    ledger: &SteamLedger,
    now: Timestamp,
    wait: Wait,
    state: &mut (ReportOutcome, Vec<SummaryJob>),
) -> crate::Result<()> {
    let (outcome, jobs) = state;
    let (account_id, match_id) = (wait.account_id, wait.match_id);

    if !user_settings::match_fetch(pool, wait.discord_id).await? {
        report_waits::delete(pool, account_id, match_id).await?;
        outcome.dropped += 1;
        return Ok(());
    }

    let mut progress = Progress {
        attempts: wait.attempts,
        next_attempt_at: wait.next_attempt_at,
        salts_at: wait.salts_at,
        metadata_at: wait.metadata_at,
    };

    let reach =
        advance(client, pool, &wait, now, ledger, &mut progress, outcome).await?;

    match reach {
        Reach::Got(body) => {
            let ready =
                metadata_players(&body).unwrap_or_default().contains(&account_id);
            let (result, label) = if ready {
                (WaitOutcome::Ready, "ready")
            } else {
                (WaitOutcome::NotInMatch, "not_in_match")
            };

            let won = report_waits::finish(
                pool,
                account_id,
                match_id,
                wait.next_attempt_at,
                result,
                now,
                &progress,
            )
            .await?;
            if !won {
                debug!(
                    match_id,
                    account_id, "deadlock: report wait taken over, nothing sent"
                );
                return Ok(());
            }

            if ready {
                if let Err(e) = (DeadlockAppEvent::MatchReady {
                    discord_id: wait.discord_id,
                    match_id,
                })
                .notify(pool)
                .await
                {
                    warn!(match_id, account_id, error = %e, "deadlock: match_ready push failed");
                }
                outcome.ready += 1;
                if let Some(job) = summary_job(api, pool, &wait, &body).await? {
                    outcome.summaries += 1;
                    jobs.push(job);
                }
            } else {
                outcome.not_in_match += 1;
            }
            info!(
                match_id,
                account_id,
                outcome = label,
                "deadlock: report wait finished"
            );
        },
        Reach::NotYet => {
            progress.attempts = wait.attempts.saturating_add(1);
            let floor = now.checked_add(MIN_GAP).unwrap_or(Timestamp::MAX);
            let base = schedule_base(wait.ended_at, wait.reported_at);
            progress.next_attempt_at =
                capped(due(base, progress.attempts).max(floor), wait.expires_at);
            report_waits::reschedule(
                pool,
                account_id,
                match_id,
                wait.next_attempt_at,
                &progress,
            )
            .await?;
        },
        Reach::Hold(retry_after) => {
            let hold = SignedDuration::try_from(retry_after)
                .unwrap_or(MAX_HOLD)
                .clamp(MIN_GAP, MAX_HOLD);
            progress.next_attempt_at = capped(
                now.checked_add(hold).unwrap_or(Timestamp::MAX),
                wait.expires_at,
            );
            report_waits::reschedule(
                pool,
                account_id,
                match_id,
                wait.next_attempt_at,
                &progress,
            )
            .await?;
            outcome.held += 1;
        },
    }

    Ok(())
}

pub async fn run(
    client: &DeadlockClient,
    api: &DeadlockApi,
    pool: &PgPool,
    ledger: &SteamLedger,
    now: Timestamp,
) -> crate::Result<(ReportOutcome, Vec<SummaryJob>)> {
    let expired = report_waits::expire(pool, now).await?;
    let lease_until = now.checked_add(LEASE).unwrap_or(Timestamp::MAX);
    let claimed = report_waits::claim_due(pool, now, lease_until, BATCH).await?;

    let outcome = ReportOutcome {
        expired,
        claimed: claimed.len(),
        ..ReportOutcome::default()
    };
    let mut state = (outcome, Vec::new());

    for wait in claimed {
        let (account_id, match_id) = (wait.account_id, wait.match_id);
        if let Err(e) = step(client, api, pool, ledger, now, wait, &mut state).await
        {
            warn!(error = ?e, match_id, account_id, "deadlock: report wait step failed");
        }
    }

    Ok(state)
}

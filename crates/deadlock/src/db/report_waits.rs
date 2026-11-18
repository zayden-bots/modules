use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use sqlx::PgPool;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Ready,
    NotInMatch,
    Expired,
}

impl WaitOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotInMatch => "not_in_match",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ready" => Some(Self::Ready),
            "not_in_match" => Some(Self::NotInMatch),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWait {
    pub account_id: u32,
    pub match_id: u64,
    pub discord_id: i64,
    pub ended_at: Timestamp,
    pub reported_at: Timestamp,
    pub expires_at: Timestamp,
    pub next_attempt_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wait {
    pub account_id: u32,
    pub match_id: u64,
    pub discord_id: i64,
    pub ended_at: Timestamp,
    pub reported_at: Timestamp,
    pub expires_at: Timestamp,
    pub next_attempt_at: Timestamp,
    pub attempts: u32,
    pub salts_at: Option<Timestamp>,
    pub metadata_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub outcome: Option<WaitOutcome>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub attempts: u32,
    pub next_attempt_at: Timestamp,
    pub salts_at: Option<Timestamp>,
    pub metadata_at: Option<Timestamp>,
}

struct WaitRecord {
    account_id: i64,
    match_id: i64,
    discord_id: i64,
    ended_at: jiff_sqlx::Timestamp,
    reported_at: jiff_sqlx::Timestamp,
    expires_at: jiff_sqlx::Timestamp,
    next_attempt_at: jiff_sqlx::Timestamp,
    attempts: i32,
    salts_at: Option<jiff_sqlx::Timestamp>,
    metadata_at: Option<jiff_sqlx::Timestamp>,
    finished_at: Option<jiff_sqlx::Timestamp>,
    outcome: Option<String>,
}

impl WaitRecord {
    fn into_wait(self) -> Option<Wait> {
        let outcome = match self.outcome.as_deref() {
            Some(text) => Some(WaitOutcome::parse(text)?),
            None => None,
        };

        Some(Wait {
            account_id: u32::try_from(self.account_id).ok()?,
            match_id: u64::try_from(self.match_id).ok()?,
            discord_id: self.discord_id,
            ended_at: self.ended_at.to_jiff(),
            reported_at: self.reported_at.to_jiff(),
            expires_at: self.expires_at.to_jiff(),
            next_attempt_at: self.next_attempt_at.to_jiff(),
            attempts: u32::try_from(self.attempts).ok()?,
            salts_at: self.salts_at.map(jiff_sqlx::Timestamp::to_jiff),
            metadata_at: self.metadata_at.map(jiff_sqlx::Timestamp::to_jiff),
            finished_at: self.finished_at.map(jiff_sqlx::Timestamp::to_jiff),
            outcome,
        })
    }
}

fn into_waits(records: Vec<WaitRecord>) -> Vec<Wait> {
    records
        .into_iter()
        .filter_map(|record| {
            let (account_id, match_id) = (record.account_id, record.match_id);
            let wait = record.into_wait();
            if wait.is_none() {
                warn!(
                    account_id,
                    match_id, "deadlock: skipping unreadable report wait"
                );
            }
            wait
        })
        .collect()
}

fn attempts_column(attempts: u32) -> i32 {
    i32::try_from(attempts).unwrap_or(i32::MAX)
}

/// `false` when a wait for this account and match already exists.
#[expect(
    trivial_casts,
    reason = "not a cast: `as T` is sqlx's bind-param type-override syntax, required because TIMESTAMPTZ has no built-in jiff mapping"
)]
pub async fn insert(pool: &PgPool, wait: &NewWait) -> sqlx::Result<bool> {
    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO deadlock_report_waits
            (account_id, match_id, discord_id, ended_at, reported_at, expires_at,
             next_attempt_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT DO NOTHING
        RETURNING match_id
        "#,
        i64::from(wait.account_id),
        wait.match_id.cast_signed(),
        wait.discord_id,
        wait.ended_at.to_sqlx() as jiff_sqlx::Timestamp,
        wait.reported_at.to_sqlx() as jiff_sqlx::Timestamp,
        wait.expires_at.to_sqlx() as jiff_sqlx::Timestamp,
        wait.next_attempt_at.to_sqlx() as jiff_sqlx::Timestamp
    )
    .fetch_optional(pool)
    .await?;

    Ok(inserted.is_some())
}

pub async fn get(
    pool: &PgPool,
    account_id: u32,
    match_id: u64,
) -> sqlx::Result<Option<Wait>> {
    let record = sqlx::query_as!(
        WaitRecord,
        r#"
        SELECT account_id,
               match_id,
               discord_id,
               ended_at AS "ended_at: jiff_sqlx::Timestamp",
               reported_at AS "reported_at: jiff_sqlx::Timestamp",
               expires_at AS "expires_at: jiff_sqlx::Timestamp",
               next_attempt_at AS "next_attempt_at: jiff_sqlx::Timestamp",
               attempts,
               salts_at AS "salts_at: jiff_sqlx::Timestamp",
               metadata_at AS "metadata_at: jiff_sqlx::Timestamp",
               finished_at AS "finished_at: jiff_sqlx::Timestamp",
               outcome
        FROM deadlock_report_waits
        WHERE account_id = $1 AND match_id = $2
        "#,
        i64::from(account_id),
        match_id.cast_signed()
    )
    .fetch_optional(pool)
    .await?;

    Ok(record.and_then(|record| into_waits(vec![record]).pop()))
}

/// Claims up to `limit` unfinished waits due at `now`, oldest due first, and leases
/// them until `lease_until`.
#[expect(
    trivial_casts,
    reason = "not a cast: `as T` is sqlx's bind-param type-override syntax, required because TIMESTAMPTZ has no built-in jiff mapping"
)]
pub async fn claim_due(
    pool: &PgPool,
    now: Timestamp,
    lease_until: Timestamp,
    limit: i64,
) -> sqlx::Result<Vec<Wait>> {
    let records = sqlx::query_as!(
        WaitRecord,
        r#"
        UPDATE deadlock_report_waits w
        SET next_attempt_at = $2
        FROM (
            SELECT account_id, match_id
            FROM deadlock_report_waits
            WHERE finished_at IS NULL AND next_attempt_at <= $1 AND expires_at > $1
            ORDER BY next_attempt_at
            LIMIT $3
            FOR UPDATE SKIP LOCKED
        ) due
        WHERE w.account_id = due.account_id AND w.match_id = due.match_id
        RETURNING w.account_id,
                  w.match_id,
                  w.discord_id,
                  w.ended_at AS "ended_at: jiff_sqlx::Timestamp",
                  w.reported_at AS "reported_at: jiff_sqlx::Timestamp",
                  w.expires_at AS "expires_at: jiff_sqlx::Timestamp",
                  w.next_attempt_at AS "next_attempt_at: jiff_sqlx::Timestamp",
                  w.attempts,
                  w.salts_at AS "salts_at: jiff_sqlx::Timestamp",
                  w.metadata_at AS "metadata_at: jiff_sqlx::Timestamp",
                  w.finished_at AS "finished_at: jiff_sqlx::Timestamp",
                  w.outcome
        "#,
        now.to_sqlx() as jiff_sqlx::Timestamp,
        lease_until.to_sqlx() as jiff_sqlx::Timestamp,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(into_waits(records))
}

#[expect(
    trivial_casts,
    reason = "not a cast: `as T` is sqlx's bind-param type-override syntax, required because TIMESTAMPTZ has no built-in jiff mapping"
)]
pub async fn expire(pool: &PgPool, now: Timestamp) -> sqlx::Result<u64> {
    let result = sqlx::query!(
        r#"
        UPDATE deadlock_report_waits
        SET finished_at = $1, outcome = 'expired'
        WHERE finished_at IS NULL AND expires_at <= $1
        "#,
        now.to_sqlx() as jiff_sqlx::Timestamp
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

#[expect(
    trivial_casts,
    reason = "not a cast: `as T` is sqlx's bind-param type-override syntax, required because TIMESTAMPTZ has no built-in jiff mapping"
)]
pub async fn reschedule(
    pool: &PgPool,
    account_id: u32,
    match_id: u64,
    leased: Timestamp,
    progress: &Progress,
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"
        UPDATE deadlock_report_waits
        SET attempts = $3, next_attempt_at = $4, salts_at = $5, metadata_at = $6
        WHERE account_id = $1 AND match_id = $2 AND finished_at IS NULL
          AND next_attempt_at = $7
        "#,
        i64::from(account_id),
        match_id.cast_signed(),
        attempts_column(progress.attempts),
        progress.next_attempt_at.to_sqlx() as jiff_sqlx::Timestamp,
        progress.salts_at.map(ToSqlx::to_sqlx) as Option<jiff_sqlx::Timestamp>,
        progress.metadata_at.map(ToSqlx::to_sqlx) as Option<jiff_sqlx::Timestamp>,
        leased.to_sqlx() as jiff_sqlx::Timestamp
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[expect(
    trivial_casts,
    reason = "not a cast: `as T` is sqlx's bind-param type-override syntax, required because TIMESTAMPTZ has no built-in jiff mapping"
)]
pub async fn finish(
    pool: &PgPool,
    account_id: u32,
    match_id: u64,
    leased: Timestamp,
    outcome: WaitOutcome,
    now: Timestamp,
    progress: &Progress,
) -> sqlx::Result<bool> {
    let finished = sqlx::query_scalar!(
        r#"
        UPDATE deadlock_report_waits
        SET finished_at = $3, outcome = $4, attempts = $5, salts_at = $6,
            metadata_at = $7
        WHERE account_id = $1 AND match_id = $2 AND finished_at IS NULL
          AND next_attempt_at = $8
        RETURNING match_id
        "#,
        i64::from(account_id),
        match_id.cast_signed(),
        now.to_sqlx() as jiff_sqlx::Timestamp,
        outcome.as_str(),
        attempts_column(progress.attempts),
        progress.salts_at.map(ToSqlx::to_sqlx) as Option<jiff_sqlx::Timestamp>,
        progress.metadata_at.map(ToSqlx::to_sqlx) as Option<jiff_sqlx::Timestamp>,
        leased.to_sqlx() as jiff_sqlx::Timestamp
    )
    .fetch_optional(pool)
    .await?;

    Ok(finished.is_some())
}

pub async fn delete(
    pool: &PgPool,
    account_id: u32,
    match_id: u64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "DELETE FROM deadlock_report_waits WHERE account_id = $1 AND match_id = $2",
        i64::from(account_id),
        match_id.cast_signed()
    )
    .execute(pool)
    .await?;

    Ok(())
}

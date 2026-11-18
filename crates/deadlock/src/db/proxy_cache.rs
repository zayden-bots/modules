use std::time::Duration;

use sqlx::PgPool;

use crate::client::CacheKind;

pub struct ProxyRow {
    pub body: String,
    pub fetched_at: jiff::Timestamp,
    pub fresh: bool,
}

pub async fn get(pool: &PgPool, key: &str) -> sqlx::Result<Option<ProxyRow>> {
    let row = sqlx::query!(
        r#"SELECT
            body,
            fetched_at as "fetched_at: jiff_sqlx::Timestamp",
            expires_at > now() AS "fresh!"
        FROM deadlock_api_cache
        WHERE key = $1"#,
        key
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| ProxyRow {
        body: row.body,
        fetched_at: row.fetched_at.to_jiff(),
        fresh: row.fresh,
    }))
}

pub async fn put(
    pool: &PgPool,
    key: &str,
    kind: CacheKind,
    client_version: Option<i32>,
    body: &str,
    ttl: Duration,
) -> sqlx::Result<jiff::Timestamp> {
    let row = sqlx::query!(
        r#"INSERT INTO deadlock_api_cache
            (key, kind, client_version, body, fetched_at, expires_at)
        VALUES ($1, $2, $3, $4, now(), now() + make_interval(secs => $5))
        ON CONFLICT (key) DO UPDATE SET
            kind = EXCLUDED.kind,
            client_version = EXCLUDED.client_version,
            body = EXCLUDED.body,
            fetched_at = now(),
            expires_at = EXCLUDED.expires_at
        RETURNING fetched_at as "fetched_at: jiff_sqlx::Timestamp""#,
        key,
        kind.as_str(),
        client_version,
        body,
        ttl.as_secs_f64()
    )
    .fetch_one(pool)
    .await?;

    Ok(row.fetched_at.to_jiff())
}

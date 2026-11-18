use std::sync::Arc;
use std::time::Duration;

use deadlock_api::{
    Bucket,
    BucketConfig,
    DeadlockApiError,
    DeadlockClient,
    Limiter,
    Priority,
};
use secrecy::{ExposeSecret, SecretString};
use serde::de::IgnoredAny;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tracing::warn;

use crate::client::CacheKind;
use crate::db::proxy_cache;
use crate::error::ProxyError;

pub const KEY_PREFIX: &str = "proxy:";

const ASSET_TTL: Duration = Duration::from_hours(24);
const CLIENT_VERSIONS_TTL: Duration = Duration::from_mins(5);
const ANALYTICS_TTL: Duration = Duration::from_hours(6);
const BUILD_TTL: Duration = Duration::from_hours(6);
const MATCH_TTL: Duration = Duration::from_hours(24 * 7);
const MAX_WAIT: Duration = Duration::from_secs(2);
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyClass {
    Asset,
    ClientVersions,
    Analytics,
    Build,
    Match,
}

impl ProxyClass {
    #[must_use]
    pub fn of(canonical: &str) -> Option<Self> {
        let (path, query) = canonical.split_once('?').unwrap_or((canonical, ""));

        if path == "/v1/assets/client-versions" {
            Some(Self::ClientVersions)
        } else if path.starts_with("/v1/assets/") {
            Some(Self::Asset)
        } else if path.starts_with("/v1/analytics/") {
            Some(Self::Analytics)
        } else if path == "/v1/builds" {
            Some(Self::Build)
        } else if path.starts_with("/v1/matches/")
            && query.split('&').any(|pair| pair == "disable_steam=true")
        {
            Some(Self::Match)
        } else {
            None
        }
    }

    #[must_use]
    pub const fn kind(self) -> CacheKind {
        match self {
            Self::Asset | Self::ClientVersions => CacheKind::Asset,
            Self::Analytics => CacheKind::Analytics,
            Self::Build => CacheKind::Build,
            Self::Match => CacheKind::Match,
        }
    }

    #[must_use]
    pub const fn ttl(self) -> Duration {
        match self {
            Self::Asset => ASSET_TTL,
            Self::ClientVersions => CLIENT_VERSIONS_TTL,
            Self::Analytics => ANALYTICS_TTL,
            Self::Build => BUILD_TTL,
            Self::Match => MATCH_TTL,
        }
    }

    #[must_use]
    pub const fn bucket(self) -> Bucket {
        match self {
            Self::Asset | Self::ClientVersions | Self::Build => Bucket::Assets,
            Self::Analytics => Bucket::Analytics,
            Self::Match => Bucket::SingleMetadata,
        }
    }
}

#[must_use]
pub fn client_version(canonical: &str) -> Option<i32> {
    let (_, query) = canonical.split_once('?')?;

    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(name, _)| *name == "client_version")
        .and_then(|(_, value)| value.parse::<u32>().ok())
        .and_then(|version| i32::try_from(version).ok())
}

/// Web's share of deadlock-api's per-IP limits; the bot processes keep
/// `Limiter::new`.
#[must_use]
pub fn proxy_limiter() -> Limiter {
    Limiter::with_configs(
        [
            (Bucket::Analytics, BucketConfig {
                capacity: 40,
                per: Duration::from_mins(1),
                reserved: 0,
            }),
            (Bucket::Assets, BucketConfig {
                capacity: 5,
                per: Duration::from_secs(1),
                reserved: 0,
            }),
            (Bucket::SingleMetadata, BucketConfig {
                capacity: 50,
                per: Duration::from_secs(10),
                reserved: 0,
            }),
        ],
        MAX_WAIT,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyBody {
    pub body: String,
    pub fetched_at: jiff::Timestamp,
    pub stale: bool,
}

#[derive(Debug)]
pub enum Lookup {
    Fresh(ProxyBody),
    Miss(Miss),
}

#[derive(Debug)]
pub struct Miss {
    key: String,
    canonical: String,
    class: ProxyClass,
    stale: Option<ProxyBody>,
}

#[derive(Debug)]
pub struct DeadlockProxy {
    client: DeadlockClient,
    pool: PgPool,
    locks: moka::future::Cache<String, Arc<Mutex<()>>>,
}

fn failure(
    key: &str,
    stale: Option<ProxyBody>,
    err: ProxyError,
) -> Result<ProxyBody, ProxyError> {
    if let Some(stale) = stale {
        warn!(error = %err, key, "deadlock-api unavailable, serving stale proxy row");
        Ok(stale)
    } else {
        Err(err)
    }
}

impl DeadlockProxy {
    pub fn new(
        http: reqwest::Client,
        pool: PgPool,
        api_key: Option<&SecretString>,
    ) -> crate::Result<Self> {
        let mut builder =
            DeadlockClient::builder().http(http).limiter(proxy_limiter());

        if let Some(key) = api_key {
            builder = builder.api_key(key.expose_secret());
        }

        Ok(Self::from_client(builder.build()?, pool))
    }

    #[must_use]
    pub fn from_client(client: DeadlockClient, pool: PgPool) -> Self {
        let locks = moka::future::Cache::builder()
            .max_capacity(10_000)
            .time_to_idle(Duration::from_secs(60))
            .build();

        Self { client, pool, locks }
    }

    pub async fn lookup(&self, canonical: &str) -> Result<Lookup, ProxyError> {
        let class = ProxyClass::of(canonical)
            .ok_or_else(|| ProxyError::NotProxied(canonical.to_owned()))?;
        let key = format!("{KEY_PREFIX}{canonical}");
        let row = proxy_cache::get(&self.pool, &key).await?;

        if let Some(row) = row {
            let fresh = row.fresh;
            let body = ProxyBody {
                body: row.body,
                fetched_at: row.fetched_at,
                stale: !fresh,
            };

            if fresh {
                return Ok(Lookup::Fresh(body));
            }

            return Ok(Lookup::Miss(Miss {
                key,
                canonical: canonical.to_owned(),
                class,
                stale: Some(body),
            }));
        }

        Ok(Lookup::Miss(Miss {
            key,
            canonical: canonical.to_owned(),
            class,
            stale: None,
        }))
    }

    pub async fn fetch(&self, miss: Miss) -> Result<ProxyBody, ProxyError> {
        let Miss { key, canonical, class, stale } = miss;

        let lock = self
            .locks
            .get_with(key.clone(), async { Arc::new(Mutex::new(())) })
            .await;
        let Ok(_guard) = tokio::time::timeout(MAX_WAIT, lock.lock()).await else {
            return failure(&key, stale, ProxyError::Busy);
        };

        if let Some(row) = proxy_cache::get(&self.pool, &key).await?
            && row.fresh
        {
            return Ok(ProxyBody {
                body: row.body,
                fetched_at: row.fetched_at,
                stale: false,
            });
        }

        match self
            .client
            .get_raw(&canonical, class.bucket(), Priority::Interactive)
            .await
        {
            Ok(body) => {
                if body.len() > MAX_BODY_BYTES {
                    return failure(&key, stale, ProxyError::TooLarge {
                        bytes: body.len(),
                    });
                }

                if let Err(e) = serde_json::from_str::<IgnoredAny>(&body) {
                    return failure(&key, stale, ProxyError::InvalidBody(e));
                }

                let client_version = if class == ProxyClass::Asset {
                    client_version(&canonical)
                } else {
                    None
                };

                let fetched_at = match proxy_cache::put(
                    &self.pool,
                    &key,
                    class.kind(),
                    client_version,
                    &body,
                    class.ttl(),
                )
                .await
                {
                    Ok(fetched_at) => fetched_at,
                    Err(e) => {
                        warn!(
                            error = %e,
                            key,
                            "failed to write deadlock proxy cache row"
                        );
                        jiff::Timestamp::now()
                    },
                };

                Ok(ProxyBody { body, fetched_at, stale: false })
            },
            Err(DeadlockApiError::NotFound) => Err(ProxyError::NotFound),
            Err(DeadlockApiError::Http { status }) if status.is_client_error() => {
                Err(ProxyError::Rejected { status: status.as_u16() })
            },
            Err(other) => failure(&key, stale, ProxyError::Upstream(other)),
        }
    }
}

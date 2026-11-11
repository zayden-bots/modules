use serenity::all::ApplicationId;
use sqlx::PgPool;
use zayden_core::as_u64;

#[derive(Debug, Clone, Copy)]
pub struct SummaryTarget {
    pub guild_id: i64,
    pub channel_id: i64,
    pub application_id: ApplicationId,
}

pub async fn select(pool: &PgPool, guild_id: i64) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "SELECT channel_id FROM deadlock_summary_channel WHERE guild_id = $1",
        guild_id
    )
    .fetch_optional(pool)
    .await
}

pub async fn upsert(
    pool: &PgPool,
    guild_id: i64,
    channel_id: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO guilds (id) VALUES ($1) ON CONFLICT (id) DO NOTHING",
        guild_id
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO deadlock_summary_channel (guild_id, channel_id)
        VALUES ($1, $2)
        ON CONFLICT (guild_id) DO UPDATE
        SET channel_id = EXCLUDED.channel_id,
            updated_at = now()
        "#,
        guild_id,
        channel_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete(pool: &PgPool, guild_id: i64) -> sqlx::Result<bool> {
    let deleted = sqlx::query!(
        "DELETE FROM deadlock_summary_channel WHERE guild_id = $1",
        guild_id
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(deleted > 0)
}

pub async fn targets_for(
    pool: &PgPool,
    discord_id: i64,
) -> sqlx::Result<Vec<SummaryTarget>> {
    let rows = sqlx::query!(
        r#"
        SELECT c.guild_id, c.channel_id,
               delivery_application(c.guild_id, c.delivery_application_id) AS "application_id!"
        FROM deadlock_summary_channel c
        JOIN deadlock_guild_members m
          ON m.guild_id = c.guild_id
         AND m.discord_id = $1
         AND m.share_summaries
        WHERE delivery_application(c.guild_id, c.delivery_application_id) IS NOT NULL
          AND NOT EXISTS (
                SELECT 1 FROM guild_app_modules gm
                WHERE gm.guild_id = c.guild_id
                  AND gm.application_id = delivery_application(c.guild_id, c.delivery_application_id)
                  AND gm.module = 'deadlock'
                  AND NOT gm.enabled
            )
        ORDER BY c.guild_id
        "#,
        discord_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SummaryTarget {
            guild_id: r.guild_id,
            channel_id: r.channel_id,
            application_id: ApplicationId::new(as_u64(r.application_id)),
        })
        .collect())
}

use sqlx::PgPool;

pub async fn dm_target(
    pool: &PgPool,
    discord_id: i64,
) -> sqlx::Result<(bool, Option<i64>)> {
    let row = sqlx::query!(
        r#"
        SELECT COALESCE(s.dm_summaries, TRUE) AS "dm_summaries!",
               serving_application(s.opt_in_guild_id, 'zayden') AS dm_application_id
        FROM (SELECT $1::bigint AS discord_id) u
        LEFT JOIN deadlock_user_settings s ON s.discord_id = u.discord_id
        "#,
        discord_id
    )
    .fetch_one(pool)
    .await?;

    Ok((row.dm_summaries, row.dm_application_id))
}

pub async fn patch_briefing(pool: &PgPool, discord_id: i64) -> sqlx::Result<bool> {
    let enabled = sqlx::query_scalar!(
        "SELECT patch_briefing FROM deadlock_user_settings WHERE discord_id = $1",
        discord_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(enabled.unwrap_or(true))
}

pub async fn set_patch_briefing(
    pool: &PgPool,
    discord_id: i64,
    enabled: bool,
    guild_id: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO deadlock_user_settings (discord_id, patch_briefing, opt_in_guild_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (discord_id) DO UPDATE
        SET patch_briefing = EXCLUDED.patch_briefing,
            opt_in_guild_id = COALESCE(
                EXCLUDED.opt_in_guild_id,
                deadlock_user_settings.opt_in_guild_id
            ),
            updated_at = now()
        "#,
        discord_id,
        enabled,
        guild_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn set_dm_summaries(
    pool: &PgPool,
    discord_id: i64,
    enabled: bool,
    guild_id: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query!(
        r#"
        INSERT INTO deadlock_user_settings (discord_id, dm_summaries, opt_in_guild_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (discord_id) DO UPDATE
        SET dm_summaries = EXCLUDED.dm_summaries,
            opt_in_guild_id = COALESCE(
                EXCLUDED.opt_in_guild_id,
                deadlock_user_settings.opt_in_guild_id
            ),
            updated_at = now()
        "#,
        discord_id,
        enabled,
        guild_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

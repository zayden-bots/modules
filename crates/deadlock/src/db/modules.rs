use sqlx::PgPool;

pub async fn enabled_guilds(
    pool: &PgPool,
    guild_ids: &[i64],
    application_id: i64,
    slot: &str,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"
        SELECT p.guild_id
        FROM guild_presence p
        WHERE p.guild_id = ANY($1)
          AND p.application_id = $2
          AND p.left_at IS NULL
          AND NOT EXISTS (
                SELECT 1 FROM guild_app_modules m
                WHERE m.guild_id = p.guild_id
                  AND m.application_id = COALESCE(serving_application(p.guild_id, $3), $2)
                  AND m.module = 'deadlock'
                  AND NOT m.enabled
            )
        ORDER BY p.guild_id
        "#,
        guild_ids,
        application_id,
        slot
    )
    .fetch_all(pool)
    .await
}

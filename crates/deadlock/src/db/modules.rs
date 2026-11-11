use serenity::all::ApplicationId;
use sqlx::PgPool;
use zayden_core::as_u64;

pub async fn enabled_guilds(
    pool: &PgPool,
    guild_ids: &[i64],
) -> sqlx::Result<Vec<(i64, ApplicationId)>> {
    let rows = sqlx::query!(
        r#"
        SELECT g.id AS "guild_id!",
               delivery_application(g.id, NULL) AS "application_id!"
        FROM unnest($1::bigint[]) AS g(id)
        WHERE delivery_application(g.id, NULL) IS NOT NULL
          AND NOT EXISTS (
                SELECT 1 FROM guild_app_modules m
                WHERE m.guild_id = g.id
                  AND m.application_id = delivery_application(g.id, NULL)
                  AND m.module = 'deadlock'
                  AND NOT m.enabled
            )
        ORDER BY g.id
        "#,
        guild_ids
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| (r.guild_id, ApplicationId::new(as_u64(r.application_id))))
        .collect())
}

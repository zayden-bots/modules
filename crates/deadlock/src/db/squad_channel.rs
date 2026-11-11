use serenity::all::ApplicationId;
use sqlx::PgPool;
use zayden_core::as_u64;

#[derive(Debug, Clone, Copy)]
pub struct SquadChannel {
    pub guild_id: i64,
    pub channel_id: i64,
    pub leaderboard: bool,
    pub leaderboard_message_id: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct LeaderboardTarget {
    pub channel: SquadChannel,
    pub application_id: ApplicationId,
}

#[derive(Debug, Clone, Copy)]
pub struct SquadTarget {
    pub guild_id: i64,
    pub channel_id: i64,
    pub application_id: ApplicationId,
}

pub async fn select(
    pool: &PgPool,
    guild_id: i64,
) -> sqlx::Result<Option<SquadChannel>> {
    sqlx::query_as!(
        SquadChannel,
        r#"
        SELECT guild_id, channel_id, leaderboard, leaderboard_message_id
        FROM deadlock_squad_channel
        WHERE guild_id = $1
        "#,
        guild_id
    )
    .fetch_optional(pool)
    .await
}

pub async fn upsert(
    pool: &PgPool,
    guild_id: i64,
    channel_id: i64,
    leaderboard: bool,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO guilds (id) VALUES ($1) ON CONFLICT (id) DO NOTHING",
        guild_id
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO deadlock_squad_channel (guild_id, channel_id, leaderboard)
        VALUES ($1, $2, $3)
        ON CONFLICT (guild_id) DO UPDATE
        SET leaderboard_message_id = CASE
                WHEN deadlock_squad_channel.channel_id <> EXCLUDED.channel_id
                THEN NULL
                ELSE deadlock_squad_channel.leaderboard_message_id
            END,
            channel_id = EXCLUDED.channel_id,
            leaderboard = EXCLUDED.leaderboard,
            updated_at = now()
        "#,
        guild_id,
        channel_id,
        leaderboard
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete(pool: &PgPool, guild_id: i64) -> sqlx::Result<bool> {
    let deleted = sqlx::query!(
        "DELETE FROM deadlock_squad_channel WHERE guild_id = $1",
        guild_id
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(deleted > 0)
}

pub async fn set_leaderboard_message(
    pool: &PgPool,
    guild_id: i64,
    message_id: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE deadlock_squad_channel SET leaderboard_message_id = $2 WHERE guild_id = $1",
        guild_id,
        message_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn targets_for(
    pool: &PgPool,
    discord_id: i64,
) -> sqlx::Result<Vec<SquadTarget>> {
    let rows = sqlx::query!(
        r#"
        SELECT c.guild_id, c.channel_id,
               delivery_application(c.guild_id, c.delivery_application_id) AS "application_id!"
        FROM deadlock_squad_channel c
        JOIN deadlock_guild_members m
          ON m.guild_id = c.guild_id
         AND m.discord_id = $1
         AND m.squad
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
        .map(|r| SquadTarget {
            guild_id: r.guild_id,
            channel_id: r.channel_id,
            application_id: ApplicationId::new(as_u64(r.application_id)),
        })
        .collect())
}

pub async fn leaderboard_guilds(
    pool: &PgPool,
) -> sqlx::Result<Vec<LeaderboardTarget>> {
    let rows = sqlx::query!(
        r#"
        SELECT c.guild_id, c.channel_id, c.leaderboard, c.leaderboard_message_id,
               delivery_application(c.guild_id, c.delivery_application_id) AS "application_id!"
        FROM deadlock_squad_channel c
        WHERE c.leaderboard
          AND delivery_application(c.guild_id, c.delivery_application_id) IS NOT NULL
          AND NOT EXISTS (
                SELECT 1 FROM guild_app_modules gm
                WHERE gm.guild_id = c.guild_id
                  AND gm.application_id = delivery_application(c.guild_id, c.delivery_application_id)
                  AND gm.module = 'deadlock'
                  AND NOT gm.enabled
            )
        ORDER BY c.guild_id
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| LeaderboardTarget {
            channel: SquadChannel {
                guild_id: r.guild_id,
                channel_id: r.channel_id,
                leaderboard: r.leaderboard,
                leaderboard_message_id: r.leaderboard_message_id,
            },
            application_id: ApplicationId::new(as_u64(r.application_id)),
        })
        .collect())
}

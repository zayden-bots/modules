use deadlock::db::announce::targets;
use sqlx::PgPool;

const ZAYDEN: i64 = 1000;

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn targets_exclude_disabled_absent_and_custom_served_guilds(pool: PgPool) {
    let rows = targets(&pool, ZAYDEN, "zayden").await.unwrap();

    let mut guilds: Vec<i64> = rows.iter().map(|row| row.guild_id).collect();
    guilds.sort_unstable();

    assert_eq!(guilds, vec![1, 2]);
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn another_application_only_sees_the_guilds_it_is_in(pool: PgPool) {
    let rows = targets(&pool, 3000, "zayden").await.unwrap();

    assert!(rows.is_empty());
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn a_switch_for_another_application_is_ignored(pool: PgPool) {
    let rows = targets(&pool, ZAYDEN, "zayden").await.unwrap();

    assert!(rows.iter().any(|row| row.guild_id == 1));
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn the_custom_application_serves_its_guild(pool: PgPool) {
    let rows = targets(&pool, 2000, "zayden").await.unwrap();

    let guilds: Vec<i64> = rows.iter().map(|row| row.guild_id).collect();

    assert_eq!(guilds, vec![6]);
}

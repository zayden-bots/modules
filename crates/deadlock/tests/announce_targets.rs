use deadlock::db::announce::targets;
use sqlx::PgPool;

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn targets_exclude_disabled_and_absent_guilds(pool: PgPool) {
    let rows = targets(&pool).await.unwrap();

    let mut guilds: Vec<i64> =
        rows.iter().map(|target| target.row.guild_id).collect();
    guilds.sort_unstable();

    assert_eq!(guilds, vec![1, 2, 6]);
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn targets_carry_the_seat_bot(pool: PgPool) {
    let rows = targets(&pool).await.unwrap();

    for target in rows {
        let expected = if target.row.guild_id == 6 { 2000 } else { 1000 };
        assert_eq!(target.application_id.get(), expected);
    }
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn a_switch_for_another_application_is_ignored(pool: PgPool) {
    let rows = targets(&pool).await.unwrap();

    assert!(rows.iter().any(|target| target.row.guild_id == 1));
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("announce_targets"))]
async fn a_custom_seat_bots_switch_decides(pool: PgPool) {
    let rows = targets(&pool).await.unwrap();

    assert!(rows.iter().any(|target| target.row.guild_id == 6));
    assert!(!rows.iter().any(|target| target.row.guild_id == 7));
}

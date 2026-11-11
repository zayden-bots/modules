use marathon::announce::MarathonAnnounceRow;
use sqlx::PgPool;

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("marathon_targets"))]
async fn targets_skip_guilds_without_the_seat_bot(pool: PgPool) {
    let targets = MarathonAnnounceRow::targets(&pool).await.unwrap();

    let found: Vec<(i64, u64)> =
        targets.iter().map(|t| (t.row.guild_id, t.application_id.get())).collect();
    assert_eq!(found, [(1, 1000)]);
}

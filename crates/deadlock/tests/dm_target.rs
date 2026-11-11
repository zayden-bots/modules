use deadlock::db::user_settings::{dm_target, set_dm_summaries, set_patch_briefing};
use sqlx::PgPool;

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("dm_target"))]
async fn dm_target_follows_the_guild_seat(pool: PgPool) {
    set_dm_summaries(&pool, 10, true, Some(6)).await.unwrap();
    set_dm_summaries(&pool, 11, false, Some(1)).await.unwrap();

    assert_eq!(dm_target(&pool, 10).await.unwrap(), (true, Some(2000)));
    assert_eq!(dm_target(&pool, 11).await.unwrap(), (false, Some(1000)));
    assert_eq!(dm_target(&pool, 12).await.unwrap(), (true, Some(1000)));
}

#[sqlx::test(migrator = "schema::MIGRATOR", fixtures("dm_target"))]
async fn opt_in_from_dms_keeps_the_guild(pool: PgPool) {
    set_dm_summaries(&pool, 10, true, Some(6)).await.unwrap();

    set_dm_summaries(&pool, 10, false, None).await.unwrap();
    assert_eq!(dm_target(&pool, 10).await.unwrap(), (false, Some(2000)));

    set_patch_briefing(&pool, 10, true, None).await.unwrap();
    assert_eq!(dm_target(&pool, 10).await.unwrap(), (false, Some(2000)));

    set_dm_summaries(&pool, 10, true, Some(1)).await.unwrap();
    assert_eq!(dm_target(&pool, 10).await.unwrap(), (true, Some(1000)));
}

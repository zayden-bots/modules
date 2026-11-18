use deadlock::db::user_settings::{
    dm_target,
    match_fetch,
    patch_briefing,
    set_dm_summaries,
    set_match_fetch,
    set_patch_briefing,
};
use sqlx::PgPool;

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn match_fetch_defaults_to_true(pool: PgPool) {
    assert!(match_fetch(&pool, 10).await.unwrap());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn set_match_fetch_round_trips(pool: PgPool) {
    set_match_fetch(&pool, 10, false, None).await.unwrap();
    assert!(!match_fetch(&pool, 10).await.unwrap());

    set_match_fetch(&pool, 10, true, None).await.unwrap();
    assert!(match_fetch(&pool, 10).await.unwrap());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn set_match_fetch_leaves_other_settings(pool: PgPool) {
    set_dm_summaries(&pool, 10, false, None).await.unwrap();
    set_patch_briefing(&pool, 10, false, None).await.unwrap();

    set_match_fetch(&pool, 10, false, None).await.unwrap();
    assert!(!dm_target(&pool, 10).await.unwrap().0);
    assert!(!patch_briefing(&pool, 10).await.unwrap());

    set_dm_summaries(&pool, 10, true, None).await.unwrap();
    assert!(!match_fetch(&pool, 10).await.unwrap());
}

use deadlock::tasks::{self, LeaderboardPayload};
use serenity::all::ApplicationId;
use sqlx::PgPool;
use zayden_core::PostPayload;

fn post(text: &str) -> PostPayload {
    PostPayload {
        channel_id: 10,
        edit_message_id: Some(20),
        message: serde_json::json!({ "content": text }),
    }
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn queueing_twice_leaves_one_leaderboard_task(pool: PgPool) {
    let app = ApplicationId::new(1000);

    tasks::queue(&pool, app, 5, post("first")).await.unwrap();
    tasks::queue(&pool, app, 5, post("second")).await.unwrap();

    let payloads = sqlx::query_scalar!(
        "SELECT payload FROM scheduled_tasks WHERE kind = $1 AND key = $2",
        tasks::LEADERBOARD,
        tasks::key(5)
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["post"]["message"]["content"], "second");
}

#[test]
fn leaderboard_payload_round_trips() {
    let payload = LeaderboardPayload { guild_id: 5, post: post("board") };

    let value = serde_json::to_value(&payload).unwrap();
    let back: LeaderboardPayload = serde_json::from_value(value).unwrap();

    assert_eq!(back.guild_id, 5);
    assert_eq!(back.post.channel_id, 10);
    assert_eq!(back.post.edit_message_id, Some(20));
    assert_eq!(back.post.message["content"], "board");
}

use std::sync::Arc;

use deadlock::DeadlockApi;
use deadlock::cron::{DeadlockMatchCron, DeadlockPatchCron};
use deadlock_api::DeadlockClient;
use serenity::all::{ApplicationId, UserId};
use sqlx::PgPool;
use zayden_app::applications::{AppKind, Slot};
use zayden_core::Identity;

const ME: Identity = Identity {
    application_id: ApplicationId::new(1),
    user_id: UserId::new(2),
    slot: Slot::Zayden,
    kind: AppKind::Default,
};

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn schedule_expression_parses(pool: PgPool) {
    let client = DeadlockClient::builder().build().unwrap();
    let api = Arc::new(DeadlockApi::from_client(client, pool));

    let job = DeadlockPatchCron::cron_job(api, ME);

    assert!(job.is_ok(), "cron schedule failed to parse: {:?}", job.err());
}

#[sqlx::test(migrator = "schema::MIGRATOR")]
async fn match_poll_schedule_expression_parses(pool: PgPool) {
    let client = DeadlockClient::builder().build().unwrap();
    let api = Arc::new(DeadlockApi::from_client(client, pool));

    let job = DeadlockMatchCron::cron_job(api, ME);

    assert!(job.is_ok(), "cron schedule failed to parse: {:?}", job.err());
}

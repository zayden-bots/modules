pub mod deeplink;
pub mod lifecycle;
pub mod reaper;
pub mod row;
pub mod tasks;

pub use reaper::JellyfinPartyReaperCron;
pub use row::{PartyGuestRow, PartyRow, PartyState};
use sqlx::PgPool;
use tracing::error;
use zayden_core::Identity;

#[derive(Debug, Default)]
pub enum Scheduled {
    #[default]
    Nothing,
    Party(Box<PartyRow>),
    Clear(i64),
}

impl Scheduled {
    pub async fn apply(self, pool: &PgPool, me: Identity) {
        match self {
            Self::Nothing => {},
            Self::Party(party) => {
                if let Err(e) = tasks::schedule(pool, me, &party).await {
                    error!(error = ?e, party_id = party.id, "could not schedule party tasks");
                }
            },
            Self::Clear(party_id) => {
                if let Err(e) = tasks::clear(pool, me, party_id).await {
                    error!(error = ?e, party_id, "could not clear party tasks");
                }
            },
        }
    }
}

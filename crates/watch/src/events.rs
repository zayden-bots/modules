use serenity::all::GuildId;
use sqlx::PgPool;
use tracing::error;
use zayden_core::{Identity, as_i64};

use crate::party::row::PartyRow;
use crate::party::tasks;

pub async fn guild_create(pool: &PgPool, guild_id: GuildId, me: Identity) {
    let parties = match PartyRow::live(pool).await {
        Ok(parties) => parties,
        Err(e) => {
            error!(error = ?e, "jellyfin: could not load live parties");
            return;
        },
    };

    let guild = as_i64(guild_id.get());

    let owned = match PartyRow::owned_ids(pool, guild, me).await {
        Ok(owned) => owned,
        Err(e) => {
            error!(error = ?e, "jellyfin: could not load owned parties");
            return;
        },
    };

    for party in
        parties.iter().filter(|p| p.guild_id == guild && owned.contains(&p.id))
    {
        // Missed reminders are deliberately not replayed — a "starts in 24 hours"
        // ping sent after the party is worse than silence.
        if let Err(e) = tasks::ensure(pool, me, party).await {
            error!(error = ?e, party_id = party.id, "could not schedule party tasks");
        }
    }
}

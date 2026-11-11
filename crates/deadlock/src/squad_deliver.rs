use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use deadlock_api::Priority;
use deadlock_api::model::Rank;
use futures::StreamExt;
use serenity::all::{ApplicationId, ChannelId, CreateComponent, MessageId};
use sqlx::PgPool;
use tracing::warn;
use zayden_core::{CronCtx, Delivered, Edited, Post};

use crate::client::DeadlockApi;
use crate::db::players;
use crate::db::squad_channel::{self, SquadTarget};
use crate::embeds;
use crate::error::Result;
use crate::rank_poll::SquadEvent;
use crate::squad::{LeaderboardEntry, order_leaderboard};

const CONCURRENCY: usize = 5;

async fn rank_names(api: &DeadlockApi) -> Vec<Rank> {
    match api.ranks(Priority::Background).await {
        Ok(fetched) => (*fetched.value).clone(),
        Err(e) => {
            warn!(error = ?e, "deadlock: rank names unavailable for squad posts");
            Vec::new()
        },
    }
}

async fn ordered_squad(
    pool: &PgPool,
    guild_id: i64,
) -> Result<Vec<LeaderboardEntry>> {
    let mut entries = players::guild_squad(pool, guild_id).await?;
    order_leaderboard(&mut entries);

    Ok(entries)
}

pub async fn leaderboard_message(
    pool: &PgPool,
    api: &DeadlockApi,
    guild_id: i64,
) -> Result<CreateComponent<'static>> {
    let entries = ordered_squad(pool, guild_id).await?;
    let ranks = rank_names(api).await;

    Ok(embeds::leaderboard_component(&entries, &ranks))
}

pub async fn deliver(
    cx: &CronCtx,
    api: &DeadlockApi,
    events: Vec<SquadEvent>,
) -> Result<usize> {
    let mut targets: HashMap<i64, Vec<SquadTarget>> = HashMap::new();
    let mut by_guild: BTreeMap<i64, (i64, ApplicationId, Vec<SquadEvent>)> =
        BTreeMap::new();

    for event in events {
        let found = match targets.entry(event.discord_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry
                .insert(squad_channel::targets_for(&cx.db, event.discord_id).await?),
        };

        for target in found.iter() {
            by_guild
                .entry(target.guild_id)
                .or_insert_with(|| {
                    (target.channel_id, target.application_id, Vec::new())
                })
                .2
                .push(event);
        }
    }

    if by_guild.is_empty() {
        return Ok(0);
    }

    let ranks = rank_names(api).await;
    let mut posts = Vec::new();

    for (guild_id, (channel_id, application_id, guild_events)) in by_guild {
        if channel_id == 0 {
            continue;
        }

        let message = embeds::squad_post(&guild_events, &ranks);
        posts.push(post(cx, guild_id, channel_id, application_id, message));
    }

    let sent = futures::stream::iter(posts)
        .buffer_unordered(CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    Ok(sent.into_iter().filter(|ok| *ok).count())
}

pub async fn post_leaderboards(cx: &CronCtx, api: &DeadlockApi) -> Result<usize> {
    let pool = &cx.db;
    let guilds = squad_channel::leaderboard_guilds(pool).await?;
    if guilds.is_empty() {
        return Ok(0);
    }

    let ranks = rank_names(api).await;
    let mut sent = 0;

    for target in guilds {
        let row = target.channel;
        let application_id = target.application_id;

        if row.channel_id == 0 {
            continue;
        }

        let entries = ordered_squad(pool, row.guild_id).await?;
        if entries.is_empty() {
            continue;
        }

        let component = embeds::leaderboard_component(&entries, &ranks);
        let channel = ChannelId::new(row.channel_id.cast_unsigned());
        let message = embeds::leaderboard_create(component);

        if let Some(message_id) = row.leaderboard_message_id.filter(|id| *id != 0) {
            let edited = cx
                .webhooks
                .edit(
                    channel,
                    application_id,
                    MessageId::new(message_id.cast_unsigned()),
                    &message,
                )
                .await;

            match edited {
                Ok(Edited::Done | Edited::Queued) => {
                    sent += 1;
                    continue;
                },
                Ok(Edited::Missing) => {},
                Err(e) => {
                    warn!(
                        error = ?e,
                        guild_id = row.guild_id,
                        "deadlock: failed to edit the squad leaderboard; posting a new one"
                    );
                },
            }
        }

        match cx.webhooks.send(channel, application_id, &message).await {
            Ok(Delivered::Webhook(id)) => {
                squad_channel::set_leaderboard_message(
                    pool,
                    row.guild_id,
                    id.get().cast_signed(),
                )
                .await?;
                sent += 1;
            },
            Ok(Delivered::Queued) => sent += 1,
            Err(e) => warn!(
                error = ?e,
                guild_id = row.guild_id,
                "deadlock: failed to post the squad leaderboard"
            ),
        }
    }

    Ok(sent)
}

async fn post(
    cx: &CronCtx,
    guild_id: i64,
    channel_id: i64,
    application_id: ApplicationId,
    message: Post<'static>,
) -> bool {
    let channel = ChannelId::new(channel_id.cast_unsigned());

    match cx.webhooks.send(channel, application_id, &message).await {
        Ok(_) => true,
        Err(e) => {
            warn!(
                error = ?e,
                guild_id,
                "deadlock: failed to post squad events; they are not retried"
            );
            false
        },
    }
}

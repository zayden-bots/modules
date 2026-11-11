use std::collections::BTreeMap;
use std::time::Duration;

use deadlock_api::Priority;
use futures::StreamExt;
use jiff::Timestamp;
use serenity::all::{ApplicationId, ChannelId, UserId};
use tracing::{error, warn};
use zayden_core::{CronCtx, Post, queue_dm};

use crate::client::DeadlockApi;
use crate::db::{guild_posts, summary_channel, user_settings};
use crate::embeds;
use crate::error::Result;
use crate::heroes::HeroIndex;
use crate::items::ItemIndex;
use crate::match_poll::SummaryJob;

pub const DM_INTERVAL: Duration = Duration::from_secs(1);

const MAX_POST_SECTIONS: usize = 4;
const CONCURRENCY: usize = 5;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct DeliveryOutcome {
    pub dms: usize,
    pub posts: usize,
}

pub async fn deliver(
    cx: &CronCtx,
    api: &DeadlockApi,
    jobs: Vec<SummaryJob>,
) -> Result<DeliveryOutcome> {
    let heroes = match api.heroes(None, Priority::Background).await {
        Ok(fetched) => HeroIndex::new(&fetched.value),
        Err(e) => {
            warn!(error = ?e, "deadlock: hero names unavailable for match summaries");
            HeroIndex::new(&[])
        },
    };
    let items = match api.items(None, Priority::Background).await {
        Ok(fetched) => ItemIndex::new(&fetched.value),
        Err(e) => {
            warn!(error = ?e, "deadlock: item names unavailable for match summaries");
            ItemIndex::new(&[])
        },
    };

    let dms = send_dms(cx, &jobs, &heroes, &items).await?;
    let posts = post_to_guilds(cx, &jobs, &heroes, &items).await?;

    Ok(DeliveryOutcome { dms, posts })
}

async fn send_dms(
    cx: &CronCtx,
    jobs: &[SummaryJob],
    heroes: &HeroIndex,
    items: &ItemIndex,
) -> Result<usize> {
    let now = Timestamp::now();
    let mut sent = 0_usize;

    for job in jobs {
        if job.discord_id == 0 {
            continue;
        }

        let (enabled, dm_application_id) =
            user_settings::dm_target(&cx.db, job.discord_id).await?;
        if !enabled {
            continue;
        }

        let post =
            embeds::summary_dm(&job.summary, heroes.get(job.summary.hero_id), items);
        let user = UserId::new(job.discord_id.cast_unsigned());
        let target = dm_application_id.map_or_else(
            || ApplicationId::new(cx.app.zayden_id),
            |id| ApplicationId::new(id.cast_unsigned()),
        );
        let offset =
            DM_INTERVAL.saturating_mul(u32::try_from(sent).unwrap_or(u32::MAX));
        let run_at = now.checked_add(offset).unwrap_or(now);

        queue_dm(
            &cx.db,
            target,
            user,
            format!("deadlock:summary:{}:{}", job.summary.match_id, job.discord_id),
            run_at,
            &post,
        )
        .await?;
        sent += 1;
    }

    Ok(sent)
}

async fn post_to_guilds(
    cx: &CronCtx,
    jobs: &[SummaryJob],
    heroes: &HeroIndex,
    items: &ItemIndex,
) -> Result<usize> {
    let pool = &cx.db;
    let mut by_match: BTreeMap<u64, Vec<&SummaryJob>> = BTreeMap::new();
    for job in jobs {
        by_match.entry(job.summary.match_id).or_default().push(job);
    }

    let mut posts = Vec::new();

    for (match_id, match_jobs) in by_match {
        let mut by_guild: BTreeMap<i64, (i64, ApplicationId, Vec<&SummaryJob>)> =
            BTreeMap::new();

        for job in match_jobs {
            let targets = summary_channel::targets_for(pool, job.discord_id).await?;

            for target in targets {
                by_guild
                    .entry(target.guild_id)
                    .or_insert_with(|| {
                        (target.channel_id, target.application_id, Vec::new())
                    })
                    .2
                    .push(job);
            }
        }

        for (guild_id, (channel_id, application_id, guild_jobs)) in by_guild {
            if channel_id == 0
                || !guild_posts::claim(pool, guild_id, match_id).await?
            {
                continue;
            }

            let sections: Vec<_> = guild_jobs
                .iter()
                .take(MAX_POST_SECTIONS)
                .map(|job| (&job.summary, job.discord_id))
                .collect();
            let message = embeds::summary_post(&sections, heroes, items);

            posts.push(post(cx, guild_id, channel_id, application_id, message));
        }
    }

    let sent = futures::stream::iter(posts)
        .buffer_unordered(CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    Ok(sent.into_iter().filter(|ok| *ok).count())
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
            error!(
                error = ?e,
                guild_id,
                "deadlock: failed to post a match summary; it is not retried"
            );
            false
        },
    }
}

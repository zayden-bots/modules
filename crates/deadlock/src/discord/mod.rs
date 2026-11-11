mod autocomplete;
mod command;

use std::sync::Arc;

use async_trait::async_trait;
use serenity::all::FullEvent;
use tracing::debug;
use zayden_app::tasks::Task;
use zayden_core::ctx::TaskCtx;
use zayden_core::error::HandlerError;
use zayden_core::{
    EventCtx,
    EventKind,
    Module,
    ModuleCtx,
    ModuleError,
    OverlapError,
    RegistryBuilder,
    as_i64,
};

use crate::worker::api_from_config;
use crate::{DeadlockApi, tasks};

pub struct DeadlockModule {
    api: Arc<DeadlockApi>,
}

impl DeadlockModule {
    pub fn from_config(cx: &ModuleCtx<'_>) -> Result<Self, ModuleError> {
        Ok(Self { api: api_from_config(&cx.worker())? })
    }
}

#[async_trait]
impl Module for DeadlockModule {
    fn id(&self) -> &'static str {
        "deadlock"
    }

    fn register(&self, b: &mut RegistryBuilder) -> Result<(), OverlapError> {
        b.add_command(command::Deadlock { api: Arc::clone(&self.api) })
            .add_autocomplete(autocomplete::Deadlock { api: Arc::clone(&self.api) });
        Ok(())
    }

    fn events(&self) -> &'static [EventKind] {
        &[EventKind::GuildMemberRemoval]
    }

    async fn on_event(
        &self,
        cx: &EventCtx<'_>,
        ev: &FullEvent,
    ) -> Result<(), HandlerError> {
        let FullEvent::GuildMemberRemoval { guild_id, user, .. } = ev else {
            return Ok(());
        };

        let removed = crate::db::guild_members::remove(
            &cx.app.db,
            as_i64(guild_id.get()),
            as_i64(user.id.get()),
        )
        .await?;

        if removed {
            debug!(%guild_id, user_id = %user.id, "Reset Deadlock opt-ins after the member left");
        }

        Ok(())
    }

    fn task_kinds(&self) -> &'static [&'static str] {
        &[tasks::LEADERBOARD]
    }

    async fn run_task(
        &self,
        cx: &TaskCtx<'_>,
        task: &Task,
    ) -> Result<(), HandlerError> {
        tasks::run(&cx.ctx.http, &cx.app.db, task).await.map_err(HandlerError::new)
    }
}

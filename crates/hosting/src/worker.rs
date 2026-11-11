use std::sync::Arc;

use secrecy::ExposeSecret;
use zayden_app::entitlement::EntitlementService;
use zayden_core::{WorkerJobResult, WorkerModule, WorkerModuleCtx};

use crate::HostingRuntime;
use crate::cron::{HostingDeleteCron, HostingExpireCron, HostingReminderCron};

pub struct HostingWorker {
    runtime: Option<Arc<HostingRuntime>>,
    entitlements: Arc<EntitlementService>,
}

impl HostingWorker {
    #[must_use]
    pub fn from_config(cx: &WorkerModuleCtx<'_>) -> Self {
        Self {
            runtime: runtime_from_config(cx),
            entitlements: Arc::clone(&cx.app.entitlements),
        }
    }
}

pub(crate) fn runtime_from_config(
    cx: &WorkerModuleCtx<'_>,
) -> Option<Arc<HostingRuntime>> {
    // Hosting needs the panel's admin-scoped key, so its client is built
    // here and never handed to the dashboard.
    cx.config.pelican.as_ref().zip(cx.config.hosting.as_ref()).map(
        |(pelican, hosting)| {
            Arc::new(HostingRuntime::new(
                cx.app.http.clone(),
                &pelican.base_url,
                pelican.api_key.expose_secret(),
                hosting.clone(),
            ))
        },
    )
}

impl WorkerModule for HostingWorker {
    fn id(&self) -> &'static str {
        "hosting"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        // Hosting is optional; without a catalog and panel credentials there
        // are no servers for these sweeps to act on.
        self.runtime.as_ref().map_or_else(Vec::new, |runtime| {
            vec![
                HostingReminderCron::worker_job(Arc::clone(runtime)),
                HostingExpireCron::worker_job(
                    Arc::clone(runtime),
                    Arc::clone(&self.entitlements),
                ),
                HostingDeleteCron::worker_job(Arc::clone(runtime)),
            ]
        })
    }
}

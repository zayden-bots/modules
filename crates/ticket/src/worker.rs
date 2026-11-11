use zayden_core::{WorkerJobResult, WorkerModule};

use crate::SupportIdleGcCron;

pub struct TicketWorker;

impl WorkerModule for TicketWorker {
    fn id(&self) -> &'static str {
        "ticket"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        vec![SupportIdleGcCron::worker_job()]
    }
}

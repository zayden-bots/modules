use zayden_core::{WorkerJobResult, WorkerModule};

use crate::StaminaCron;

pub struct GamblingWorker;

impl WorkerModule for GamblingWorker {
    fn id(&self) -> &'static str {
        "gambling"
    }

    fn jobs(&self) -> Vec<WorkerJobResult> {
        vec![StaminaCron::worker_job()]
    }
}

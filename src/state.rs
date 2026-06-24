use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{Duration, Local};
use crate::types::{Status, Worker};

pub type SharedState = Arc<Mutex<AppState>>;

#[derive(Default)]
pub struct AppState {
    pub workers: HashMap<String, Worker>
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, name: String) {
        let worker = Worker {
            name: name.clone(),
            status: Status::Online,
            last_heartbeat: Local::now(),
            job_id: None,
        };
        self.workers.insert(name, worker);
    }

    pub fn heartbeat(&mut self, name: &str) -> bool {
        match self.workers.get_mut(name) {
            Some(worker) => {
                worker.last_heartbeat = Local::now();
                worker.status = Status::Online;
                true
            },
            None => false,
        }
    }

    pub fn list_workers(&self) -> Vec<Worker> {
        self.workers.values().cloned().collect()
    }

    pub fn online_workers(&self) -> u16 {
        self.workers.values().filter(|worker| matches!(worker.status, Status::Online)).count() as u16
    }

    pub fn reap(&mut self, timeout: Duration) {
        let now = Local::now();
        for worker in self.workers.values_mut() {
            if now - worker.last_heartbeat > timeout {
                worker.status = Status::Offline;
            }
        }
    }
}
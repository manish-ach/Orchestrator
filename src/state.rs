use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{Duration, Local};
use crate::types::{Job, JobStatus, Status, Worker};

pub type SharedState = Arc<Mutex<AppState>>;

#[derive(Default)]
pub struct AppState {
    pub workers: HashMap<String, Worker>,
    pub jobs: Vec<Job>,
    pub next_job_id: u32,
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

    pub fn trigger_pipeline(&mut self) {
        let stages = [
            ("build", "echo building && sleep 2"),
            ("test", "echo testing && sleep 1"),
            ("deploy", "echo deploy && sleep 2"),
        ];

        for (stage, command) in stages {
            let job = Job {
                id: self.next_job_id,
                stage_name: stage.to_string(),
                command: command.to_string(),
                status: JobStatus::Pending,
                assigned_worker: None
            };
            self.jobs.push(job);
            self.next_job_id += 1;
        }
    }

    pub fn list_jobs(&self) -> Vec<Job> {
        self.jobs.clone()
    }

    pub fn claim_job(&mut self, worker_name: String) -> Option<Job> {
        let job = self.jobs
            .iter_mut()
            .find(|j| matches!(j.status, JobStatus::Pending))?;

        job.status = JobStatus::Running;
        job.assigned_worker = Some(worker_name);

        let claim = job.clone();
        Some(claim)
    }
}
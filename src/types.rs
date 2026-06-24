use chrono::{DateTime, Local};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize)]
pub struct Worker {
    pub name: String,
    pub status: Status,
    pub last_heartbeat: DateTime<Local>,
    pub job_id: Option<u16>,
}

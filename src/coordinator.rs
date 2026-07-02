use std::sync::{Arc, Mutex};
use crate::api;
use crate::state::{AppState, SharedState};

const HEARTBEAT_TIMEOUT_SECS: i64 = 5;
const REAPER_INTERVAL_SECS: u64 = 2;

pub async fn execute(port: u16) {
    let state: SharedState = Arc::new(Mutex::new(AppState::new()));
    spawn_reaper(state.clone());

    let app = api::router(state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Port unavailable");

    axum::serve(listener, app)
        .await
        .expect("Axum server error");
}

fn spawn_reaper(state: SharedState) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(REAPER_INTERVAL_SECS));
        loop {
            ticker.tick().await;
            state
                .lock()
                .unwrap()
                .reap(chrono::Duration::seconds(HEARTBEAT_TIMEOUT_SECS))
        }
    });
}
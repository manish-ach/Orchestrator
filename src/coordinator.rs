use std::sync::Arc;

use crate::api;
use crate::forgejo;
use crate::store::{SharedStore, Store};

const REPO_REFRESH_SECS: u64 = 120;

pub async fn execute(port: u16) {
    let store = match Store::connect().await {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("Could not reach the backing stores: {error}");
            eprintln!("Start them with:  docker compose up -d");
            std::process::exit(1);
        }
    };

    if let Err(error) = store.reconcile_queue().await {
        println!("Queue reconcile failed: {error}");
    }
    spawn_repo_registry(store.clone());

    let app = api::router(store);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Port unavailable");

    println!("Coordinator listening on {addr} (Postgres + Redis connected)");
    axum::serve(listener, app)
        .await
        .expect("Axum server error");
}

// Refreshes Forgejo metadata for every registered repo so the dashboard
// stays fresh without hammering Forgejo on each 3s poll.
fn spawn_repo_registry(store: SharedStore) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        loop {
            let remotes = store.repo_remotes().await.unwrap_or_default();
            for remote in &remotes {
                match forgejo::fetch_repo(&client, remote).await {
                    Ok(repo) => {
                        if let Err(error) = store.upsert_repo(&repo).await {
                            println!("Repo save failed for {remote}: {error}");
                        }
                    }
                    Err(error) => println!("Repo refresh failed for {remote}: {error}"),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(REPO_REFRESH_SECS)).await;
        }
    });
}

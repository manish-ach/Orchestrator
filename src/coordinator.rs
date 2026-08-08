use std::sync::Arc;

use crate::api;
use crate::forgejo;
use crate::schedule;
use crate::store::{SharedStore, Store};

const REPO_REFRESH_SECS: u64 = 120;
const RECONCILE_INTERVAL_SECS: u64 = 5;

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
    spawn_reconciler(store.clone());
    spawn_scheduler(store.clone());

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

// Steals work back from dead workers: drains their personal queues and
// requeues their running jobs so a lost laptop doesn't strand a run.
fn spawn_reconciler(store: SharedStore) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(RECONCILE_INTERVAL_SECS));
        loop {
            ticker.tick().await;
            if let Err(error) = store.reconcile().await {
                println!("Reconcile failed: {error}");
            }
        }
    });
}

// Fires pipelines that declare a top-level `schedule:` cron expression.
//
// Ticks on the minute rather than on a fixed interval from boot: a job asked
// for 02:00 should run at 02:00, not at 02:00 plus however long ago the
// coordinator happened to start. Each candidate claims its minute in Postgres
// before running, so a restart mid-minute cannot double-fire and two
// coordinators pointed at one database cannot both win.
fn spawn_scheduler(store: SharedStore) {
    tokio::spawn(async move {
        loop {
            sleep_until_next_minute().await;
            let now = chrono::Local::now();
            // truncate to the minute: this is the value the claim is keyed on
            let minute = now.timestamp() / 60;

            let repos = store.list_repos().await.unwrap_or_default();
            for repo in repos {
                for pipeline in &repo.pipelines {
                    let Some(expr) = pipeline.schedule.as_deref() else { continue };
                    let cron = match schedule::parse(expr) {
                        Ok(c) => c,
                        Err(why) => {
                            // say so once a minute rather than silently never
                            // running — a typo'd schedule is otherwise invisible
                            println!("Bad schedule '{expr}' in {}/{}: {why}", repo.name, pipeline.file);
                            continue;
                        }
                    };
                    if !cron.matches(now) {
                        continue;
                    }
                    let key = format!("{}::{}", repo.name, pipeline.file);
                    match store.claim_schedule(&key, minute).await {
                        Ok(false) => continue, // someone else already ran this minute
                        Err(error) => {
                            println!("Schedule claim failed for {key}: {error}");
                            continue;
                        }
                        Ok(true) => {}
                    }
                    match api::start_scheduled_run(&store, &repo, pipeline).await {
                        Ok(id) => println!(
                            "Scheduled run {id}: {} / {} ({expr})",
                            repo.name, pipeline.name
                        ),
                        Err(why) => println!("Scheduled run for {key} failed: {why}"),
                    }
                }
            }
        }
    });
}

/// Sleep to the top of the next minute, so ticks land on :00 seconds.
async fn sleep_until_next_minute() {
    use chrono::Timelike;
    let now = chrono::Local::now();
    let secs = 60 - now.second() as u64;
    let millis = secs * 1000 - (now.timestamp_subsec_millis() as u64).min(999);
    tokio::time::sleep(std::time::Duration::from_millis(millis.max(1))).await;
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

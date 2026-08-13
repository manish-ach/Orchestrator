//ORCHESTRATOR - WORK COMPLETE 22 shrawan YAY -NAY more work
use clap::{Parser, Subcommand};

mod coordinator;
mod types;
mod worker;
mod api;
mod store;
mod pipeline;
mod forgejo;
mod insights;
mod schedule;
mod tui;
mod ui;

#[derive(Subcommand)]
enum Module {
    ///Executes the Coordinator module, pass --port arg with port number [default: 8019]
    Coordinator {
        #[arg(long, default_value_t = 8019)]
        port: u16,
    },
    ///Loads the Worker module, pass --name arg with the worker name
    Worker {
        #[arg(long)]
        name: String,
        ///Coordinator address, e.g. http://192.168.1.10:8080 [env: COORDINATOR_URL]
        #[arg(long)]
        coordinator: Option<String>,
        ///Command executor address on this machine [env: EXECUTOR_URL]
        #[arg(long)]
        executor: Option<String>,
        ///Comma-separated capability tags, e.g. heavy,docker [env: WORKER_TAGS]
        #[arg(long)]
        tags: Option<String>,
        ///Force the full-screen dashboard even when stdout is not a terminal
        #[arg(long, conflicts_with = "no_tui")]
        tui: bool,
        ///Plain timestamped log lines instead of the dashboard. Servers and
        ///systemd units want this; it is also the automatic choice when stdout
        ///is not a terminal, so you rarely need to pass it.
        #[arg(long)]
        no_tui: bool,
    },
}

///The triple slash comment does appear as hint text on runtime
#[derive(Parser)]
#[command(about, version, long_about = None)]
struct Args {
    ///Choose the module
    #[command(subcommand)]
    module: Module,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    match args.module {
        Module::Coordinator { port } => coordinator::execute(port).await,
        Module::Worker { name, coordinator, executor, tags, tui, no_tui } => {
            let mode = ui::Mode::resolve(tui, no_tui);
            worker::run(name, coordinator, executor, tags, mode).await
        }
    }
}

use clap::{Parser, Subcommand};

mod coordinator;
mod types;
mod worker;

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
        Module::Worker { name } => worker::register(name).await,
    }
}

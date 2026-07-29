mod daemon;
mod paths;
mod repository;
mod runner;
mod schedule;
mod scheduler;
mod service;
mod socket;
mod types;

#[tokio::main]
async fn main() {
    if let Err(error) = daemon::run().await {
        eprintln!("pi-cron-daemon: {error}");
        std::process::exit(1);
    }
}

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use fedimint_core::config::load_from_file;
use mint_client::{Client, UserClientConfig};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Config {
    workdir: PathBuf,
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args();
    if let Some(ref arg) = args.nth(1) {
        if arg.as_str() == "version-hash" {
            println!("{}", env!("GIT_HASH"));
            return;
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let opts = Config::parse();
    let cfg_path = opts.workdir.join("client.json");
    let db_path = opts.workdir.join("client.db");
    let cfg: UserClientConfig = load_from_file(&cfg_path);
    let db = fedimint_rocksdb::RocksDb::open(db_path)
        .expect("Error opening DB")
        .into();

    let client = Arc::new(Client::new(cfg.clone(), db, Default::default()));

    match clientd::run_clientd(client, 8081).await {
        Ok(_) => {}
        Err(e) => {
            tracing::error!("Error running clientd: {}", e);
        }
    };
}

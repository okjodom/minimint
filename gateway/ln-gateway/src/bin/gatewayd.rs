use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use fedimint_api::{
    core::{
        LEGACY_HARDCODED_INSTANCE_ID_LN, LEGACY_HARDCODED_INSTANCE_ID_MINT,
        LEGACY_HARDCODED_INSTANCE_ID_WALLET,
    },
    module::registry::ModuleDecoderRegistry,
    task::TaskGroup,
};
use fedimint_server::modules::{
    ln::common::LightningDecoder, mint::common::MintDecoder, wallet::common::WalletDecoder,
};
use ln_gateway::{
    client::{DynGatewayClientBuilder, RocksDbFactory, StandardGatewayClientBuilder},
    gateway::Gateway,
    rpc::lnrpc_client::{DynLnRpcClientFactory, NetworkLnRpcClientFactory},
};
use tracing::{error, info};
use url::Url;

#[derive(Parser)]
pub struct GatewayOpts {
    /// Path to folder containing gateway config and data files
    #[arg(long = "data-dir", env = "FM_GATEWAY_DATA_DIR")]
    pub data_dir: PathBuf,

    /// Gateway webserver bind address
    #[arg(long = "bind-addr", env = "FM_GATEWAY_BIND_ADDR")]
    pub bind_address: SocketAddr,

    /// Public URL from which the webserver API is reachable
    #[arg(long = "announce-addr", env = "FM_GATEWAY_ANNOUNCE_ADDR")]
    pub announce_address: Url,

    /// webserver authentication password
    #[arg(long = "password", env = "FM_GATEWAY_PASSWORD")]
    pub password: String,
}

/// Fedimint Gateway Binary
///
/// This binary runs a webserver with an API that can be used by Fedimint clients to request routing of payments
/// through the Lightning Network. It uses a Gateway Lightning RPC client to communicate with a Lightning node.
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let mut args = std::env::args();

    if let Some(ref arg) = args.nth(1) {
        if arg.as_str() == "version-hash" {
            println!("{}", env!("GIT_HASH"));
            return Ok(());
        }
    }

    // Read configurations
    let GatewayOpts {
        data_dir,
        bind_address,
        announce_address,
        password,
    } = GatewayOpts::parse();

    info!(
        "Starting gateway with these configs \n data dir: {:?}, bind addr: {}, announce addr {}",
        data_dir, bind_address, announce_address
    );

    // Create federation client builder
    let client_builder: DynGatewayClientBuilder = StandardGatewayClientBuilder::new(
        data_dir.clone(),
        RocksDbFactory.into(),
        announce_address,
    )
    .into();

    // Create task group for controlled shutdown of the gateway
    let task_group = TaskGroup::new();

    // Create a lightning rpc client factory
    let lnrpc_factory: DynLnRpcClientFactory = NetworkLnRpcClientFactory::default().into();

    // Create module decoder registry
    let decoders = ModuleDecoderRegistry::from_iter([
        (LEGACY_HARDCODED_INSTANCE_ID_LN, LightningDecoder.into()),
        (LEGACY_HARDCODED_INSTANCE_ID_MINT, MintDecoder.into()),
        (LEGACY_HARDCODED_INSTANCE_ID_WALLET, WalletDecoder.into()),
    ]);

    // Create gateway instance
    let gateway = Gateway::new(decoders, lnrpc_factory, client_builder, task_group.clone()).await;

    if let Err(e) = gateway.run(bind_address, password).await {
        task_group.shutdown_join_all().await?;

        error!("Gateway stopped with error: {}", e);
        return Err(e.into());
    }

    Ok(())
}

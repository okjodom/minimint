#![allow(where_clauses_object_safety)] // https://github.com/dtolnay/async-trait/issues/228
extern crate fedimint_core;

use std::fs;
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use config::io::{read_server_config, PLAINTEXT_PASSWORD};
use config::ServerConfig;
use fedimint_aead::random_salt;
use fedimint_core::config::ServerModuleInitRegistry;
use fedimint_core::core::ModuleInstanceId;
use fedimint_core::db::Database;
use fedimint_core::epoch::ConsensusItem;
use fedimint_core::module::{ApiEndpoint, ApiEndpointContext, ApiError, ApiRequestErased};
use fedimint_core::task::TaskGroup;
use fedimint_core::util::write_new;
use fedimint_logging::{LOG_CONSENSUS, LOG_NET_API};
use futures::FutureExt;
use jsonrpsee::server::{PingConfig, RpcServiceBuilder, ServerBuilder, ServerHandle};
use jsonrpsee::types::ErrorObject;
use jsonrpsee::RpcModule;
use tracing::{error, info};

use crate::config::api::{ConfigGenApi, ConfigGenSettings};
use crate::config::io::{write_server_config, SALT_FILE};
use crate::metrics::initialize_gauge_metrics;
use crate::net::api::RpcHandlerCtx;
use crate::net::connect::TlsTcpConnector;

pub mod envs;
pub mod metrics;

pub mod atomic_broadcast;

/// The actual implementation of consensus
pub mod consensus;

/// Provides interfaces for ACID-compliant data store backends
pub mod db;

/// Networking for mint-to-mint and client-to-mint communiccation
pub mod net;

/// Fedimint toplevel config
pub mod config;

/// Implementation of multiplexed peer connections
pub mod multiplexed;

/// How long to wait before timing out client connections
const API_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(60);

/// Has the context necessary for serving API endpoints
///
/// Returns the specific `State` the endpoint requires and the
/// `ApiEndpointContext` which all endpoints can access.
#[async_trait]
pub trait HasApiContext<State> {
    async fn context(
        &self,
        request: &ApiRequestErased,
        id: Option<ModuleInstanceId>,
    ) -> (&State, ApiEndpointContext<'_>);
}

pub async fn run(
    data_dir: PathBuf,
    settings: ConfigGenSettings,
    db: Database,
    version_hash: String,
    module_init_registry: &ServerModuleInitRegistry,
    task_group: TaskGroup,
) -> anyhow::Result<()> {
    let cfg = match get_config(&data_dir).await? {
        Some(cfg) => cfg,
        None => {
            run_config_gen(
                data_dir,
                settings,
                db.clone(),
                version_hash,
                task_group.make_subgroup(),
            )
            .await?
        }
    };

    let decoders = module_init_registry.decoders_strict(
        cfg.consensus
            .modules
            .iter()
            .map(|(id, config)| (*id, &config.kind)),
    )?;

    let db = db.with_decoders(decoders);

    initialize_gauge_metrics(&db).await;

    let (engine, api_handler) =
        consensus::spawn_api_and_build_engine(cfg, db, module_init_registry.clone(), &task_group)
            .await?;

    engine.run().await?;

    api_handler.stop().await;

    info!(target: LOG_CONSENSUS, "Shutting down tasks");

    task_group.shutdown();

    Ok(())
}

pub async fn get_config(data_dir: &Path) -> anyhow::Result<Option<ServerConfig>> {
    // Attempt get the config with local password, otherwise start config gen
    if let Ok(password) = fs::read_to_string(data_dir.join(PLAINTEXT_PASSWORD)) {
        return Ok(Some(read_server_config(&password, data_dir.to_owned())?));
    }

    Ok(None)
}

pub async fn run_config_gen(
    data_dir: PathBuf,
    settings: ConfigGenSettings,
    db: Database,
    version_hash: String,
    mut task_group: TaskGroup,
) -> anyhow::Result<ServerConfig> {
    info!(target: LOG_CONSENSUS, "Starting config gen");

    initialize_gauge_metrics(&db).await;

    let (cfg_sender, mut cfg_receiver) = tokio::sync::mpsc::channel(1);

    let config_gen = ConfigGenApi::new(
        settings.clone(),
        db.clone(),
        cfg_sender,
        &mut task_group,
        version_hash.clone(),
    );

    let mut rpc_module = RpcHandlerCtx::new_module(config_gen);

    attach_endpoints(&mut rpc_module, config::api::server_endpoints(), None);

    let handler = spawn_api("config-gen", &settings.api_bind, rpc_module, 10).await;

    let cfg = cfg_receiver.recv().await.expect("should not close");

    handler.stop().await;

    // TODO: Make writing password optional
    write_new(data_dir.join(PLAINTEXT_PASSWORD), &cfg.private.api_auth.0)?;
    write_new(data_dir.join(SALT_FILE), random_salt())?;
    write_server_config(
        &cfg,
        data_dir.clone(),
        &cfg.private.api_auth.0,
        &settings.registry,
    )?;

    Ok(cfg)
}

/// Spawns an API server
///
/// `force_shutdown` runs the API in a new runtime that the
/// `FedimintApiHandler` can force to shutdown, otherwise the task cannot
/// easily be killed.
async fn spawn_api<T>(
    name: &'static str,
    api_bind: &SocketAddr,
    module: RpcModule<RpcHandlerCtx<T>>,
    max_connections: u32,
) -> FedimintApiHandler {
    let handle = ServerBuilder::new()
        .max_connections(max_connections)
        .enable_ws_ping(PingConfig::new().ping_interval(Duration::from_secs(10)))
        .set_rpc_middleware(RpcServiceBuilder::new().layer(metrics::jsonrpsee::MetricsLayer))
        .build(&api_bind.to_string())
        .await
        .context(format!("Bind address: {api_bind}"))
        .context(format!("API name: {name}"))
        .expect("Could not build API server")
        .start(module);
    info!(target: LOG_NET_API, "Starting api on ws://{api_bind}");

    FedimintApiHandler { handle }
}

/// Attaches `endpoints` to the `RpcModule`
fn attach_endpoints<State, T>(
    rpc_module: &mut RpcModule<RpcHandlerCtx<T>>,
    endpoints: Vec<ApiEndpoint<State>>,
    module_instance_id: Option<ModuleInstanceId>,
) where
    T: HasApiContext<State> + Sync + Send + 'static,
    State: Sync + Send + 'static,
{
    for endpoint in endpoints {
        let path = if let Some(module_instance_id) = module_instance_id {
            // This memory leak is fine because it only happens on server startup
            // and path has to live till the end of program anyways.
            Box::leak(format!("module_{}_{}", module_instance_id, endpoint.path).into_boxed_str())
        } else {
            endpoint.path
        };
        // Check if paths contain any abnormal characters
        if path.contains(|c: char| !matches!(c, '0'..='9' | 'a'..='z' | '_')) {
            panic!("Constructing bad path name {path}");
        }

        // Another memory leak that is fine because the function is only called once at
        // startup
        let handler: &'static _ = Box::leak(endpoint.handler);

        rpc_module
            .register_async_method(path, move |params, rpc_state| async move {
                let params = params.one::<serde_json::Value>()?;
                let rpc_context = &rpc_state.rpc_context;

                // Using AssertUnwindSafe here is far from ideal. In theory this means we could
                // end up with an inconsistent state in theory. In practice most API functions
                // are only reading and the few that do write anything are atomic. Lastly, this
                // is only the last line of defense
                AssertUnwindSafe(tokio::time::timeout(API_ENDPOINT_TIMEOUT, async {
                    let request = serde_json::from_value(params)
                        .map_err(|e| ApiError::bad_request(e.to_string()))?;
                    let (state, context) = rpc_context.context(&request, module_instance_id).await;

                    (handler)(state, context, request).await
                }))
                .catch_unwind()
                .await
                .map_err(|_| {
                    error!(
                        target: LOG_NET_API,
                        path, "API handler panicked, DO NOT IGNORE, FIX IT!!!"
                    );
                    ErrorObject::owned(500, "API handler panicked", None::<()>)
                })?
                .map_err(|tokio::time::error::Elapsed { .. }| {
                    // TODO: find a better error for this, the error we used before:
                    // jsonrpsee::core::Error::RequestTimeout
                    // was moved to be client-side only
                    ErrorObject::owned(-32000, "Request timeout", None::<()>)
                })?
                .map_err(|e| ErrorObject::owned(e.code, e.message, None::<()>))
            })
            .expect("Failed to register async method");
    }
}

pub struct FedimintApiHandler {
    handle: ServerHandle,
}

impl FedimintApiHandler {
    /// Attempts to stop the API
    pub async fn stop(self) {
        let _ = self.handle.stop();
        self.handle.stopped().await;
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

pub fn check_auth(context: &mut ApiEndpointContext) -> ApiResult<()> {
    if !context.has_auth() {
        Err(ApiError::unauthorized())
    } else {
        Ok(())
    }
}

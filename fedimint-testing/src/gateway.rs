use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use fedimint_client::module::gen::ClientModuleGenRegistry;
use fedimint_client::Client;
use fedimint_core::config::FederationId;
use fedimint_core::db::mem_impl::MemDatabase;
use fedimint_core::db::Database;
use fedimint_core::module::registry::ModuleDecoderRegistry;
use fedimint_core::task::TaskGroup;
use lightning::routing::gossip::RoutingFees;
use ln_gateway::client::GatewayClientBuilder;
use ln_gateway::lnrpc_client::ILnRpcClient;
use ln_gateway::rpc::rpc_client::GatewayRpcClient;
use ln_gateway::rpc::rpc_server::run_webserver;
use ln_gateway::rpc::{ConnectFedPayload, FederationInfo};
use ln_gateway::Gateway;
use tempfile::TempDir;
use tokio::sync::RwLock;
use tracing::info;
use url::Url;

use crate::federation::FederationTest;
use crate::fixtures::test_dir;
use crate::ln::LightningTest;

/// Fixture for creating a gateway
pub struct GatewayTest {
    tg: TaskGroup,
    /// Password for the RPC
    pub password: String,
    client_builder: GatewayClientBuilder,
    db: Database,
    listen: SocketAddr,
    api: Url,
    /// Handle of the running gateway
    gateway: Option<Gateway>,
    /// Temporary dir that stores the gateway config
    _config_dir: Option<TempDir>,
    /// Handle to the webserver task group
    webserver: Option<TaskGroup>,
    scid_to_federation: Arc<RwLock<BTreeMap<u64, FederationId>>>,
    clients: Arc<RwLock<BTreeMap<FederationId, Client>>>,
    ln_client: Arc<dyn ILnRpcClient>,
}

impl GatewayTest {
    /// RPC client for communicating with the gateway admin API
    pub async fn get_rpc(&self) -> GatewayRpcClient {
        GatewayRpcClient::new(self.api.clone(), self.password.clone())
    }

    /// Removes a client from the gateway
    pub async fn remove_client(&self, fed: &FederationTest) -> Client {
        if let Some(gw) = self.gateway.clone() {
            gw.remove_client(fed.id())
                .await
                .expect("Failed to remove client")
        } else {
            panic!("Gateway not running")
        }
    }

    pub async fn select_client(&self, federation_id: FederationId) -> Client {
        if let Some(gw) = self.gateway.clone() {
            gw.select_client(federation_id)
                .await
                .expect("Failed to select client")
        } else {
            panic!("Gateway not running")
        }
    }

    /// Connects to a new federation and stores the info
    pub async fn connect_fed(&mut self, fed: &FederationTest) -> FederationInfo {
        let invite_code = fed.invite_code().to_string();
        let rpc = self.get_rpc().await;
        rpc.connect_federation(ConnectFedPayload { invite_code })
            .await
            .unwrap()
    }

    pub fn get_gateway_id(&self) -> secp256k1::PublicKey {
        self.gateway.clone().unwrap().gateway_id
    }

    pub async fn shutdown_gateway(mut self) -> Self {
        info!("Shutting down gateway");
        self.webserver.take().unwrap().shutdown().await;
        self.gateway.take().unwrap();
        self
    }

    pub async fn run_gateway(mut self) -> Self {
        let listen = self.listen;
        let fees = RoutingFees {
            base_msat: 0,
            proportional_millionths: 0,
        };
        // Create gateway with the client created from `route_htlcs`
        let gateway = Gateway::new(
            self.ln_client.clone(),
            self.client_builder.clone(),
            fees,
            self.db.clone(),
            self.api.clone(),
            self.clients.clone(),
            self.scid_to_federation.clone(),
            self.tg.clone(),
        )
        .await
        .unwrap();

        let webserver = run_webserver(self.password.clone(), listen, gateway.clone())
            .await
            .expect("Failed to start webserver");
        self.gateway = Some(gateway);
        self.webserver = Some(webserver);
        self
    }

    pub(crate) async fn new(
        base_port: u16,
        password: String,
        lightning: Box<dyn LightningTest>,
        decoders: ModuleDecoderRegistry,
        registry: ClientModuleGenRegistry,
    ) -> Self {
        let listen: SocketAddr = format!("127.0.0.1:{base_port}").parse().unwrap();
        let api: Url = format!("http://{listen}").parse().unwrap();

        let (path, _config_dir) = test_dir(&format!("gateway-{}", rand::random::<u64>()));
        let client_builder: GatewayClientBuilder =
            GatewayClientBuilder::new(path.clone(), registry, 0);

        let db = Database::new(MemDatabase::new(), decoders.clone());
        let clients = Arc::new(RwLock::new(BTreeMap::new()));
        let scid_to_federation = Arc::new(RwLock::new(BTreeMap::new()));

        // Create the stream to route HTLCs. We cannot create the Gateway until the
        // stream to the lightning node has been setup.
        let mut tg = TaskGroup::new();
        let (stream, ln_client) = lightning.route_htlcs(&mut tg).await.unwrap();

        let gw_test = Self {
            tg: tg.clone(),
            password,
            client_builder,
            db,
            listen,
            api,
            _config_dir,
            gateway: None,
            webserver: None,
            scid_to_federation: scid_to_federation.clone(),
            ln_client: ln_client.clone(),
            clients: clients.clone(),
        };

        // Spawn new thread to listen for HTLCs
        tg.spawn("Subscribe to intercepted HTLCs", move |handle| async move {
            Gateway::handle_htlc_stream(stream, ln_client, handle, scid_to_federation, clients)
                .await;
        })
        .await;

        gw_test.run_gateway().await
    }
}

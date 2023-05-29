use std::net::SocketAddr;
use std::str::FromStr;

use anyhow::anyhow;
use clap::Parser;
use fedimint_core::task::TaskGroup;
use ln_gateway::gatewaylnrpc::gateway_lightning_server::{
    GatewayLightning, GatewayLightningServer,
};
use ln_gateway::gatewaylnrpc::{
    EmptyRequest, GetNodeInfoResponse, GetRouteHintsResponse, PayInvoiceRequest,
    PayInvoiceResponse, RouteHtlcRequest, RouteHtlcResponse,
};
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Server;
use tonic::Status;
use tracing::debug;
use zebedee_rust::payments::{pay_invoice, Payment};
use zebedee_rust::{StdResp, ZebedeeClient};

#[derive(Parser)]
pub struct ZbdGatewayExtensionOpts {
    /// ZBD extension service listen address
    #[arg(long = "listen", env = "FM_ZBD_EXTENSION_LISTEN_ADDRESS")]
    pub listen: SocketAddr,

    #[arg(long = "api-key", env = "ZBD_API_KEY")]
    pub api_key: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Read configurations
    let ZbdGatewayExtensionOpts { listen, api_key } = ZbdGatewayExtensionOpts::parse();

    let service = ZbdGatewayExtension::new(api_key)
        .await
        .expect("Failed to create zbd rpc service");

    debug!(
        "Starting gateway-zbd-extension with listen address : {}",
        listen
    );

    Server::builder()
        .add_service(GatewayLightningServer::new(service))
        .serve(listen)
        .await
        .map_err(|e| ZebedeeGatewayError::Error(anyhow!("Failed to start server, {:?}", e)))?;

    Ok(())
}

#[allow(dead_code)]
pub struct ZbdGatewayExtension {
    client: ZebedeeClient,
    task_group: TaskGroup,
    api_key: String,
}

impl ZbdGatewayExtension {
    pub async fn new(api_key: String) -> Result<Self, ZebedeeGatewayError> {
        Ok(Self {
            client: ZebedeeClient::new().apikey(api_key.clone()).build(),
            task_group: TaskGroup::new(),
            api_key,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct PayInvoicePayload {
    description: String,
    internal_id: String,
    invoice: String,
    callback_url: String,
    amount: u32,
}

#[tonic::async_trait]
impl GatewayLightning for ZbdGatewayExtension {
    async fn get_node_info(
        &self,
        _request: tonic::Request<EmptyRequest>,
    ) -> Result<tonic::Response<GetNodeInfoResponse>, Status> {
        // TODO: source node info from zebedee apis

        // Return info from this zebedee node as placeholder:
        // https://amboss.space/node/03d6b14390cd178d670aa2d57c93d9519feaae7d1e34264d8bbb7932d47b75a50d
        return Ok(tonic::Response::new(GetNodeInfoResponse {
            pub_key: PublicKey::from_str(
                "03d6b14390cd178d670aa2d57c93d9519feaae7d1e34264d8bbb7932d47b75a50d",
            )
            .unwrap()
            .serialize()
            .to_vec(),
            alias: "zlnd1".to_string(),
        }));
    }

    async fn get_route_hints(
        &self,
        _request: tonic::Request<EmptyRequest>,
    ) -> Result<tonic::Response<GetRouteHintsResponse>, Status> {
        // return empty
        unimplemented!()
    }

    async fn pay_invoice(
        &self,
        request: tonic::Request<PayInvoiceRequest>,
    ) -> Result<tonic::Response<PayInvoiceResponse>, tonic::Status> {
        let payment = Payment {
            invoice: request.into_inner().invoice,
            ..Default::default()
        };

        let StdResp { success, data, .. } = pay_invoice(self.client.clone(), payment)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        if success && data.is_some() {
            return Ok(tonic::Response::new(PayInvoiceResponse {
                preimage: data.unwrap().preimage.unwrap().into_bytes(),
            }));
        }

        return Err(Status::internal("Failed to pay invoice"));
    }

    type RouteHtlcsStream = ReceiverStream<Result<RouteHtlcResponse, Status>>;

    async fn route_htlcs(
        &self,
        _request: tonic::Request<tonic::Streaming<RouteHtlcRequest>>,
    ) -> Result<tonic::Response<Self::RouteHtlcsStream>, Status> {
        unimplemented!()
    }
}

#[derive(Debug, Error)]
pub enum ZebedeeGatewayError {
    #[error("Zebedee Gateway Extension Error : {0:?}")]
    Error(#[from] anyhow::Error),
}

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use fedimint_core::task::TaskGroup;
use fedimint_core::Amount;
use fedimint_ln_common::PrunedInvoice;
use secp256k1::PublicKey;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot::Sender;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tracing::info;
use zebedee_rust::payments::{Payment, PaymentInvoiceResponse};
use zebedee_rust::ZebedeeClient;

use super::{
    send_htlc_to_webhook, ILnRpcClient, LightningRpcError, RouteHtlcStream, WebhookClient,
};
use crate::gateway_lnrpc::{
    EmptyResponse, GetNodeInfoResponse, GetRouteHintsResponse, InterceptHtlcRequest,
    InterceptHtlcResponse, PayInvoiceRequest, PayInvoiceResponse,
};
use crate::rpc::rpc_webhook_server::run_webhook_server;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AlbyPayResponse {
    amount: u64,
    description: String,
    destination: String,
    fee: u64,
    payment_hash: Vec<u8>,
    payment_preimage: Vec<u8>,
    payment_request: String,
}

#[derive(Clone)]
pub struct GatewayZbdClient {
    client: ZebedeeClient,
    bind_addr: SocketAddr,
    api_key: String,
    pub outcomes: Arc<Mutex<BTreeMap<u64, Sender<InterceptHtlcResponse>>>>,
}

impl GatewayZbdClient {
    pub async fn new(
        bind_addr: SocketAddr,
        api_key: String,
        outcomes: Arc<Mutex<BTreeMap<u64, Sender<InterceptHtlcResponse>>>>,
    ) -> Self {
        info!("Gateway configured to connect to Zebedee at \n address: {bind_addr:?}");
        let client = ZebedeeClient::new().apikey(api_key.clone()).build();
        Self {
            client,
            bind_addr,
            api_key,
            outcomes,
        }
    }
}

impl fmt::Debug for GatewayZbdClient {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AlbyClient")
    }
}

#[async_trait]
impl ILnRpcClient for GatewayZbdClient {
    /// Returns the public key of the lightning node to use in route hint
    async fn info(&self) -> Result<GetNodeInfoResponse, LightningRpcError> {
        let mainnet = "mainnet";
        let alias = "zlnd1";
        let pub_key = PublicKey::from_str(
            "03d6b14390cd178d670aa2d57c93d9519feaae7d1e34264d8bbb7932d47b75a50d",
        )
        .unwrap();
        let pub_key = pub_key.serialize().to_vec();

        return Ok(GetNodeInfoResponse {
            pub_key,
            alias: alias.to_string(),
            network: mainnet.to_string(),
        });
    }

    /// We can probably just use the Alby node pubkey here?
    /// SCID is the short channel ID mapping to the federation
    async fn routehints(
        &self,
        _num_route_hints: usize,
    ) -> Result<GetRouteHintsResponse, LightningRpcError> {
        todo!()
    }

    /// Pay an invoice using the alby api
    /// Pay needs to be idempotent, this is why we need lookup payment,
    /// would need to do something similar with Alby
    async fn pay(
        &self,
        request: PayInvoiceRequest,
    ) -> Result<PayInvoiceResponse, LightningRpcError> {
        let payment = Payment {
            invoice: request.invoice,
            ..Default::default()
        };

        let response: PaymentInvoiceResponse =
            self.client.pay_invoice(&payment).await.map_err(|e| {
                LightningRpcError::FailedPayment {
                    failure_reason: e.to_string(),
                }
            })?;

        if response.success && response.data.is_some() {
            let data = response.data.unwrap();
            let preimage = data.preimage.unwrap().into_bytes();
            return Ok(PayInvoiceResponse { preimage });
        }

        Err(LightningRpcError::FailedPayment {
            failure_reason: "Failed to pay invoice".to_string(),
        })
    }

    // FIXME: deduplicate implementation with pay
    async fn pay_private(
        &self,
        _invoice: PrunedInvoice,
        _max_delay: u64,
        _max_fee: Amount,
    ) -> Result<PayInvoiceResponse, LightningRpcError> {
        todo!()

        // Ok(PayInvoiceResponse { preimage })
    }

    /// Returns true if the lightning backend supports payments without full
    /// invoices
    fn supports_private_payments(&self) -> bool {
        false
    }

    async fn route_htlcs<'a>(
        self: Box<Self>,
        task_group: &mut TaskGroup,
    ) -> Result<(RouteHtlcStream<'a>, Arc<dyn ILnRpcClient>), LightningRpcError> {
        const CHANNEL_SIZE: usize = 100;
        let (gateway_sender, gateway_receiver) =
            mpsc::channel::<Result<InterceptHtlcRequest, tonic::Status>>(CHANNEL_SIZE);

        let new_client =
            Arc::new(Self::new(self.bind_addr, self.api_key.clone(), self.outcomes.clone()).await);

        run_webhook_server(
            self.bind_addr,
            task_group,
            gateway_sender.clone(),
            WebhookClient::Zbd(*self),
        )
        .await
        .map_err(|_| LightningRpcError::FailedToRouteHtlcs {
            failure_reason: "Failed to start webhook server".to_string(),
        })?;

        Ok((Box::pin(ReceiverStream::new(gateway_receiver)), new_client))
    }

    async fn complete_htlc(
        &self,
        htlc: InterceptHtlcResponse,
    ) -> Result<EmptyResponse, LightningRpcError> {
        send_htlc_to_webhook(&self.outcomes, htlc).await?;
        Ok(EmptyResponse {})
    }
}

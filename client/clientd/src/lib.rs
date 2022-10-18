use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, RequestParts};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::BoxError;
use axum::{Extension, Router, Server};
use bitcoin::hashes::hex::ToHex;
use bitcoin::secp256k1::rand;
use bitcoin::Transaction;
use fedimint_api::{Amount, OutPoint, TieredMulti, TransactionId};
use fedimint_core::modules::wallet::txoproof::TxOutProof;
use mint_client::mint::{NoteIssuanceRequests, SpendableNote};
use mint_client::ClientError;
use mint_client::{Client, UserClientConfig};
use rand::rngs::OsRng;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tower::ServiceBuilder;
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::{info, Level};

#[derive(Error, Debug)]
pub enum ClientdError {
    #[error("Client error: {0}")]
    ClientError(#[from] ClientError),
    #[error("Fatal server error, action reqired")]
    ServerError,
}

impl IntoResponse for ClientdError {
    fn into_response(self) -> Response {
        let payload = json!({ "error": self.to_string(), });
        let code = match self {
            ClientdError::ClientError(_) => StatusCode::BAD_REQUEST,
            ClientdError::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Result::<(), _>::Err((code, axum::Json(payload))).into_response()
    }
}
/// struct to process wait_block_height request payload
#[derive(Deserialize, Serialize)]
pub struct WaitBlockHeightPayload {
    pub height: u64,
}

/// Struct used with the axum json-extractor to proccess the peg_in request payload
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct PegInPayload {
    pub txout_proof: TxOutProof,
    pub transaction: Transaction,
}

#[derive(Deserialize, Serialize)]
pub struct SpendPayload {
    pub amount: Amount,
}

#[derive(Deserialize, Serialize)]
pub struct InfoResponse {
    notes: TieredNoteCount,
    pending: PendingResponse,
}

impl InfoResponse {
    pub fn new(
        notes: TieredMulti<SpendableNote>,
        active_issuances: Vec<(OutPoint, NoteIssuanceRequests)>,
    ) -> Self {
        Self {
            notes: notes.into(),
            pending: PendingResponse::new(active_issuances),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct PendingResponse {
    transactions: Vec<PendingTransaction>,
}

impl PendingResponse {
    pub fn new(active_issuances: Vec<(OutPoint, NoteIssuanceRequests)>) -> Self {
        let transactions: Vec<PendingTransaction> = active_issuances
            .iter()
            .map(|(out_point, cfd)| PendingTransaction {
                txid: out_point.txid.to_hex(),
                qty: cfd.coin_count(),
                value: cfd.coin_amount(),
            })
            .collect();
        Self { transactions }
    }
}

#[derive(Deserialize, Serialize)]
pub struct PegInAddressResponse {
    pub peg_in_address: bitcoin::Address,
}

#[derive(Deserialize, Serialize)]
pub struct PegInOutResponse {
    pub txid: TransactionId,
}

#[derive(Deserialize, Serialize)]
pub struct SpendResponse {
    pub notes: TieredMulti<SpendableNote>,
}

/// Represents an e-cash tier (msat by convention) grouped with a quantity of notes
///
/// e.g { tier: 1000, quantity: 10 } means 10x notes worth 1000msat each
#[derive(Serialize, Deserialize, Clone, Debug)]
struct TieredCount {
    tier: u64,
    quantity: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TieredNoteCount(Vec<TieredCount>);

/// Holds a pending transaction with the txid, the quantity of notes and the value
///
/// e.g { txid: xxx, qty: 10, value: 1 } is a pending transaction 'worth' 10btc
/// notice that this are ALL pending transactions not only the ['Accepted'](fedimint_core::outcome::TransactionStatus) ones !
#[derive(Deserialize, Serialize)]
pub struct PendingTransaction {
    txid: String,
    qty: usize,
    value: Amount,
}

pub async fn call<P>(url: String, endpoint: String, params: &P)
where
    P: Serialize + ?Sized,
{
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{}{}", url, endpoint))
        .json(params)
        .send()
        .await
        .expect("Failed to send request");

    println!("status: {}", response.status());
    let txt = &response.text().await.unwrap();
    let val: serde_json::Value = serde_json::from_str(txt).expect("failed to parse response");
    let formatted = serde_json::to_string_pretty(&val).expect("failed to format response");
    println!("{}", formatted);
}

// We need our own `Json` extractor that customizes the error from `axum::Json`
pub struct Json<T>(pub T);

#[async_trait]
impl<B, T> FromRequest<B> for Json<T>
where
    T: DeserializeOwned + Send,
    B: axum::body::HttpBody + Send,
    B::Data: Send,
    B::Error: Into<BoxError>,
{
    type Rejection = (StatusCode, axum::Json<Value>);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req).await {
            Ok(value) => Ok(Self(value.0)),
            // convert the error from `axum::Json` into whatever we want
            Err(rejection) => {
                let payload = json!({
                    "error": rejection.to_string(),
                });

                let code = match rejection {
                    JsonRejection::JsonDataError(_) => StatusCode::UNPROCESSABLE_ENTITY,
                    JsonRejection::JsonSyntaxError(_) => StatusCode::BAD_REQUEST,
                    JsonRejection::MissingJsonContentType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                Err((code, axum::Json(payload)))
            }
        }
    }
}

#[macro_export(local_inner_macros)]
macro_rules! json_success {
    () => {
        {
       let body = serde_json::json!({
            "data": {}
       });

       Ok(axum::Json(body))
        }
    };
    ($payload:expr) => {
        {
       let body = serde_json::json!({
            "data": $payload
       });

       Ok(axum::Json(body))
    }
    };
}

impl From<TieredMulti<SpendableNote>> for TieredNoteCount {
    fn from(tiered_multi: TieredMulti<SpendableNote>) -> Self {
        let all_tiered: Vec<TieredCount> = tiered_multi
            .iter_tiers()
            .map(|(tier, n)| TieredCount {
                quantity: n.len() as u64,
                tier: tier.milli_sat,
            })
            .collect();
        TieredNoteCount(all_tiered)
    }
}

// TODO: generalize for all types of clients?
struct State {
    client: Arc<Client<UserClientConfig>>,
    fetch_tx: Sender<()>,
    rng: OsRng,
}

pub async fn run_clientd(client: Arc<Client<UserClientConfig>>, port: u16) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(1024);
    let rng = OsRng;

    let shared_state = Arc::new(State {
        client: Arc::clone(&client),
        fetch_tx: tx,
        rng,
    });
    let app = Router::new()
        .route("/get_info", post(info))
        .route("/get_pending", post(pending))
        .route("/get_new_peg_in_address", post(new_peg_in_address))
        .route("/wait_block_height", post(wait_block_height))
        .route("/peg_in", post(peg_in))
        .route("/spend", post(spend))
        .layer(
            ServiceBuilder::new()
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(DefaultMakeSpan::new().include_headers(true))
                        .on_request(DefaultOnRequest::new().level(Level::INFO))
                        .on_response(DefaultOnResponse::new().level(Level::INFO)),
                )
                .layer(Extension(shared_state)),
        );

    let fetch_client = Arc::clone(&client);
    tokio::spawn(async move {
        while rx.recv().await.is_some() {
            fetch(Arc::clone(&fetch_client)).await;
        }
    });

    Server::bind(&format!("127.0.0.1:{}", port).parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
    Ok(())
}

/// Handler for "get_info", returns all the clients holdings and pending transactions
async fn info(Extension(state): Extension<Arc<State>>) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    json_success!(InfoResponse::new(
        client.coins(),
        client.list_active_issuances(),
    ))
}

/// Handler for "get_pending", returns the clients pending transactions
async fn pending(
    Extension(state): Extension<Arc<State>>,
) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    json_success!(PendingResponse::new(client.list_active_issuances()))
}

async fn new_peg_in_address(
    Extension(state): Extension<Arc<State>>,
) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    let mut rng = state.rng;
    json_success!(PegInAddressResponse {
        peg_in_address: client.get_new_pegin_address(&mut rng)
    })
}

async fn wait_block_height(
    Extension(state): Extension<Arc<State>>,
    Json(payload): Json<WaitBlockHeightPayload>,
) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    client.await_consensus_block_height(payload.height).await;
    json_success!("done")
}

async fn peg_in(
    Extension(state): Extension<Arc<State>>,
    payload: Json<PegInPayload>,
) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    let fetch_signal = &state.fetch_tx;
    let mut rng = state.rng;
    let txout_proof = payload.0.txout_proof;
    let transaction = payload.0.transaction;
    let txid = client.peg_in(txout_proof, transaction, &mut rng).await?;
    info!("Started peg-in {}", txid.to_hex());
    fetch_signal
        .send(())
        .await
        .map_err(|_| ClientdError::ServerError)?;
    json_success!(PegInOutResponse { txid })
}

async fn spend(
    Extension(state): Extension<Arc<State>>,
    payload: Json<SpendPayload>,
) -> Result<impl IntoResponse, ClientdError> {
    let client = &state.client;
    let rng = state.rng;

    let notes = client.spend_ecash(payload.0.amount, rng).await?;
    json_success!(SpendResponse { notes })
}

async fn fetch(client: Arc<Client<UserClientConfig>>) {
    //TODO: log txid or error (handle unwrap)
    let batch = client.fetch_all_coins().await;
    for item in batch.iter() {
        match item {
            Ok(out_point) => {
                //TODO: Log event
                info!("fetched notes: {}", out_point);
            }
            Err(err) => {
                //TODO: Log event
                info!("error fetching notes: {}", err);
            }
        }
    }
}

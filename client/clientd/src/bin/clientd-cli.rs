use bitcoin::Transaction;
use clap::{Parser, Subcommand};
use clientd::{call, PegInPayload, SpendPayload, WaitBlockHeightPayload};
use fedimint_api::Amount;
use fedimint_core::modules::wallet::txoproof::TxOutProof;
use mint_client::utils::{from_hex, parse_fedimint_amount};

#[derive(Parser)]
#[command(author, version, about = "a json-rpc cli application")]
struct Cli {
    /// The address of the clientd server
    #[clap(short, long, default_value = "http://localhost:8081")]
    url: String,
    /// print unformatted json
    #[arg(long = "raw", short = 'r')]
    raw_json: bool,
    /// call JSON-2.0 RPC method
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Print the latest git commit hash this bin. was build with
    VersionHash,
    /// rpc-method: info()
    Info,
    /// rpc-method: pending()
    Pending,
    /// rpc-method: pegin_address()
    NewPegInAddress,
    /// rpc-method: wait_block_height()
    #[clap(arg_required_else_help = true)]
    WaitBlockHeight { height: u64 },
    /// rpc-method peg_in()
    PegIn {
        /// The TxOutProof which was created from sending BTC to the pegin-address
        #[arg(value_parser = from_hex::<TxOutProof>)]
        txout_proof: TxOutProof,
        /// The Bitcoin Transaction
        #[arg(value_parser = from_hex::<Transaction>)]
        transaction: Transaction,
    },
    //TODO: Encode coins and/or give option (flag) to get them raw
    /// rpc-method_ spend()
    Spend {
        /// A minimint (ecash) amount
        #[arg(value_parser = parse_fedimint_amount)]
        amount: Amount,
    },
}
#[tokio::main]
async fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::VersionHash => {
            //TODO: add a type to display the cli results
            println!("{}", env!("GIT_HASH"));
        }
        Commands::Info => {
            call(args.url, String::from("/get_info"), "").await;
        }
        Commands::Pending => {
            call(args.url, String::from("/get_pending"), "").await;
        }
        Commands::NewPegInAddress => {
            call(args.url, String::from("/get_new_peg_in_address"), "").await;
        }
        Commands::WaitBlockHeight { height } => {
            let params = WaitBlockHeightPayload { height };
            call(args.url, String::from("/wait_block_height"), &params).await;
        }
        Commands::PegIn {
            txout_proof,
            transaction,
        } => {
            let params = PegInPayload {
                txout_proof,
                transaction,
            };
            call(args.url, String::from("/peg_in"), &params).await;
        }
        Commands::Spend { amount } => {
            let params = SpendPayload { amount };
            call(args.url, String::from("/spend"), &params).await;
        }
    }
}

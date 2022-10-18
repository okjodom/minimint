use anyhow::Result;
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
            print_response(
                call(args.url, String::from("/get_info"), "").await,
                args.raw_json,
            );
        }
        Commands::Pending => {
            print_response(
                call(args.url, String::from("/get_pending"), "").await,
                args.raw_json,
            );
        }
        Commands::NewPegInAddress => {
            print_response(
                call(args.url, String::from("/get_new_peg_in_address"), "").await,
                args.raw_json,
            );
        }
        Commands::WaitBlockHeight { height } => {
            let params = WaitBlockHeightPayload { height };
            print_response(
                call(args.url, String::from("/wait_block_height"), &params).await,
                args.raw_json,
            );
        }
        Commands::PegIn {
            txout_proof,
            transaction,
        } => {
            let params = PegInPayload {
                txout_proof,
                transaction,
            };
            print_response(
                call(args.url, String::from("/peg_in"), &params).await,
                args.raw_json,
            );
        }
        Commands::Spend { amount } => {
            let params = SpendPayload { amount };
            print_response(
                call(args.url, String::from("/spend"), &params).await,
                args.raw_json,
            );
        }
    }
}

fn print_response(response: Result<serde_json::Value>, raw: bool) {
    match response {
        Ok(json) => {
            if raw {
                serde_json::to_writer(std::io::stdout(), &json).unwrap();
            } else {
                serde_json::to_writer_pretty(std::io::stdout(), &json).unwrap();
            }
        }
        Err(err) => eprintln!("{}", err),
    }
}

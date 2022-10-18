#!/usr/bin/env bash
# Runs a CLI-based integration test

set -euxo pipefail
export RUST_LOG=info

source ./scripts/setup-tests.sh
./scripts/start-fed.sh

#start two test clientd instances sharing a client
$FM_CLIENTD $FM_CFG_DIR &
echo $! >> $FM_PID_FILE
await_server_on_port 8081 # first test clientd
await_server_on_port 8082 # second test clientd

#### BEGIN TESTS ####
[[ $($FM_CLIENTD_CLI info | jq -r 'has("data")') = true ]]
[[ $($FM_CLIENTD_CLI pending | jq -r 'has("data")') = true ]]
[[ $($FM_CLIENTD_CLI new-peg-in-address | jq -r 'has("data")') = true ]]
ADDR=$($FM_CLIENTD_CLI new-peg-in-address | jq -r '.data.peg_in_address');

#for peg-in we need the TxOutProof and a Transaction
TX_ID="$($FM_BTC_CLIENT sendtoaddress $ADDR 0.00001)"
$FM_BTC_CLIENT generatetoaddress 11 "$($FM_BTC_CLIENT getnewaddress)"

#wait until valid (also test the wait-block-height endpoint)
EXPECTED_BLOCK_HEIGHT="$(( $($FM_BTC_CLIENT getblockchaininfo | jq -r '.blocks') - $(get_finality_delay) ))"
[[ $($FM_CLIENTD_CLI wait-block-height  $EXPECTED_BLOCK_HEIGHT | jq -r '.data') = "done" ]]

TXOUT_PROOF="$($FM_BTC_CLIENT gettxoutproof "[\"$TX_ID\"]")"
TRANSACTION="$($FM_BTC_CLIENT getrawtransaction $TX_ID)"

#perform peg-in
[[ $($FM_CLIENTD_CLI peg-in $TXOUT_PROOF $TRANSACTION| jq -r 'has("data")') = true ]]

#until we can check th (peg-in) tx status we just have to sleep to wait for fetch
#the sleep here is unneccessary high but a I want to be sure to avoid unnecessary CI failure
sleep 20 
#spend
ECASH=$($FM_CLIENTD_CLI spend 1000);
[[ $(echo $ECASH | jq -r 'has("data")') = true ]]

#!/usr/bin/env bash
# Starts the gateway daemon

echo "Configuring gateway..."

export FM_GATEWAY_DATA_DIR=$FM_CFG_DIR/gateway
export FM_GATEWAY_BIND_ADDR="127.0.0.1:10175"
export FM_GATEWAY_ANNOUNCE_ADDR="http://127.0.0.1:10175"
export FM_GATEWAY_PASSWORD="theresnosecondbest"

echo "Running gatewayd"
$FM_BIN_DIR/gatewayd

Intention of this fork is to expiriment to have Torq (https://github.com/lncapital/torq/) as the gateway LN node for Fedimint. Torq could "proxy" any node that is connected to Torq and expose a single grpc interface to fedi gateway.

### Setup

- Setup and run Torq with a correct branch (image) (with fedi grpc)
  - In folder running-torq there is a docker compose and torq.conf file to run
  - Just run "docker compose up" in the folder
  - localhost UI port is 3003 and UI password 1234
  - In torq ui select the correct network that you are going to use from the globe icon
- You might have to change FM_GATEWAY_LIGHTNING_ADDR ip adderss to your LAN ip in devimint/src/vars.rs
- Setup and run Devimint ./docs/dev-env.md
- Close the process of lnd gateway
  -  kill $(ps aux | grep -w -v grep | grep "gatewayd lnd" | awk '{print $2}')
- Connect Torq to the LND node of Devimint (address of the node localhost:11009)
  - In torq ui "add node"
  - Address of the node localhost:11009
  - Connection files are in temp folder created by Devimint
  - Enable "intercept htlcs" while adding the node!
- In Torq settings, set the FEDI gateway node to the LND node and enable the FEDI grpc 
- It should now be possible to use the cln-gateway that is actually Torq (and the LND node)

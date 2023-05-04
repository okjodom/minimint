# Guardian UI

## Run the UI

From root repo directory:

- `cd guardian-ui`
- `nix develop .#fedimint-ui`
- `yarn` (only needs done on first init)
- `yarn start`
- Browse to url in console logs

## Run UI with Mint

- `nix develop .#fedimint-ui`
- `./scripts/run-ui.sh`
- Observe logs for "Federation 1 is using FM_API_URL ws://127.0.0.1:18184" or similar
- `cd guardian-ui`
- `REACT_APP_FM_CONFIG_API=ws://127.0.0.1:18174 yarn start` (**note** the port may be different, compare to `run-ui.sh` logs)
- The `run-ui.sh` script starts two fedimints so you can run a Guardian UI for each if desired.

## Run Tests

TODO

## CI and misc.

TODO

## NOTE

To the 🦀 devs, we're sorry for the javascript.

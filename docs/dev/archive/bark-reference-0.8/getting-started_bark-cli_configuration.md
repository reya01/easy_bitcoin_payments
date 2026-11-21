> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Configure Bark

> Configure the `bark` CLI through environment variables and `config.toml`: logging, data directory, Ark server, chain source, refresh threshold, and exit margin.

## Environment variables

Available environment variables:

* `BARK_VERBOSE` (`1` or `true`): set verbose logging
* `BARK_QUIET` (`1` or `true`): set no logging
* `BARK_DATADIR`: set the data directory to use
* `BARK_LOG`: set a `env_logger`-style filter for logging (for example, `info` or `info,bark=trace`)

> [!TIP]
>
> **Default data directory**
>
> To check your default data directory, run the following command:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark help
```

## Configuration file

Your Bark data directory contains a `config.toml` file. It supports the following configuration variables:

* `server_address`: the address of your Ark server
* `esplora_address`: the address of the Esplora HTTP REST server to use
* `bitcoind_address`: the address of the `bitcoind` RPC server to use
* `bitcoind_cookiefile`: the path to the `bitcoind` rpc cookie file
* `bitcoind_user`: the `bitcoind` RPC username
* `bitcoind_pass`: the `bitcoind` RPC password
* `vtxo_refresh_expiry_threshold`: the number of blocks before expiry to refresh VTXOs
* `vtxo_exit_margin`: an upper limit of the number of blocks we expect to need to safely exit the VTXOs
* `htlc_recv_claim_delta`: the number of blocks to claim a HTLC-recv VTXO
* `fallback_fee_rate`: a fallback feerate to use in sat/kWu when we fail to retrieve a feerate from the configured `bitcoind`/Esplora connection
* `round_tx_required_confirmations`: the number of confirmations required before considering a round transaction fully confirmed

For the full definition, see the [configuration struct](https://gitlab.com/ark-bitcoin/bark/-/blob/master/bark/src/config.rs#L75) in the bark repository.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
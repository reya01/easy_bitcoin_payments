> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Connection details

> Second's public Ark server and Esplora endpoints for mainnet and signet. Copy them into bark, the Bark SDK, or barkd.

> [tab] Mainnet
>
> ```text Ark server theme={"theme":{"light":"github-light","dark":"github-dark"}}
> https://ark.second.tech
> ```
>
> ```text Esplora theme={"theme":{"light":"github-light","dark":"github-dark"}}
> https://mempool.second.tech/api
> ```

> [tab] Signet
>
> ```text Ark server theme={"theme":{"light":"github-light","dark":"github-dark"}}
> https://ark.signet.2nd.dev
> ```
>
> ```text Esplora theme={"theme":{"light":"github-light","dark":"github-dark"}}
> https://esplora.signet.2nd.dev
> ```

## Configure a chain source

With the Bark CLI, pass only `--ark`. The CLI falls back to the Esplora endpoint for your network when you don't pass `--esplora` or `--bitcoind`. `barkd` does the same when you create a wallet without a `chain_source`. The Bark SDK requires an explicit chain source, so set the Esplora endpoint when you configure the wallet. To use your own `bitcoind` node instead, use the Bark CLI, `barkd`, or the Rust SDK. The SDK's other language bindings support Esplora only.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
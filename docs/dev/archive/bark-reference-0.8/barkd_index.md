> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Barkd

> Barkd is Second's reference Bark wallet daemon. Run it on your server and drive it from any language via a local REST API.

Barkd runs as a daemon alongside your other services and exposes a local REST API, so any backend in your stack can send and receive bitcoin over Ark, Lightning, and on-chain.

Barkd is the right fit when you run infrastructure and need to manage bitcoin server-side:

* Self-hosted node platforms (Umbrel, Start9, RaspiBlitz, etc.)
* Webshops and e-commerce platforms accepting payments
* Exchanges managing user deposits and withdrawals
* Payment processors settling merchant payments
* Backend services automating payouts, subscriptions, or reconciliation

> [!TIP]
>
> If you're building bitcoin apps for mobile or desktop, [try the Bark SDK](https://second.tech/docs/bark-sdk.md).

## Get started

> [card] [Install](https://second.tech/docs/barkd/install.md)
>
> Download binaries or compile `barkd` from source.

> [card] [Start the daemon](https://second.tech/docs/barkd/start.md)
>
> Run `barkd` with the right data directory, bind address, and CORS settings.

> [card] [Authenticate](https://second.tech/docs/barkd/authenticate.md)
>
> Retrieve and rotate the bearer token `barkd` uses to authorize REST calls.

> [card] [REST API](https://second.tech/docs/barkd/api-reference/wallet/create-a-wallet.md)
>
> Full reference for every `barkd` REST endpoint.

> [card] [Clients](https://second.tech/docs/barkd/clients.md)
>
> Typed TypeScript and C# packages that wrap the REST API.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
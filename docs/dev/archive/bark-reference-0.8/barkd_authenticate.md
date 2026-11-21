> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Authenticate

> How Barkd generates an auth token and how to retrieve or rotate it with `barkd secret show` and `barkd secret refresh`.

All REST calls to `barkd` must include a bearer token. `barkd` generates one automatically on first start and stores it in the data directory.

**1. Show the token**

Print the current bearer token:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
barkd --datadir <path-to-bark-data> secret show
```

**2. Rotate the token**

To rotate the token, run:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
barkd --datadir <path-to-bark-data> secret refresh
```

You can also supply your own secret instead of letting `barkd` generate one. Pass it as 64 hex characters (32 bytes), and `barkd` derives the bearer token from it:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
barkd --datadir <path-to-bark-data> secret refresh --secret <64-hex-character-secret>
```

Both commands print the new token. Restart `barkd` for the new token to take effect.

## Next steps

With the daemon running and a token in hand, you can start driving `barkd` over its REST API.

> [card] [REST API](https://second.tech/docs/barkd/api-reference/wallet/create-a-wallet.md)
>
> Full reference for every `barkd` REST endpoint.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
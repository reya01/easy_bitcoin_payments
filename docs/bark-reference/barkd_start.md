> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Start the daemon

> Start the Barkd daemon, point it at a data directory, set the bind address for its REST API, and list the CORS origins allowed to call it from a browser.

Run `barkd` and point it at a data directory. On first start, it initializes the data directory and generates an [auth token](https://second.tech/docs/barkd/authenticate.md).

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
barkd --datadir <path-to-bark-data>
```

The API listens on `http://localhost:3000/api/v1` by default. A Swagger UI is served at `/swagger-ui` and the OpenAPI spec at `/api-docs/openapi.json`.

## Common flags

* `--datadir <path>` — where `barkd` stores wallet data and its auth secret. Defaults to `~/.bark`. Also settable via `BARKD_DATADIR`.
* `--host <host>` and `--port <port>` — bind address for the REST API. Also settable via `BARKD_BIND_HOST` and `BARKD_BIND_PORT`.
* `--allowed-origins <origins>` — comma-separated list of CORS origins allowed to call the API from a browser. Cross-origin requests are denied by default. Also settable via `BARKD_ALLOWED_ORIGINS`.

> [!WARNING]
>
> `barkd` serves plain HTTP and binds to loopback by default. Every request carries the bearer token, and anyone with the token can control the wallet. To expose the API beyond loopback, put it behind a reverse proxy that terminates TLS.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Create a wallet

> Creates a new wallet with the specified Ark server and chain source configuration. Fails if a wallet already exists. Returns the wallet fingerprint on success.



## OpenAPI

````yaml https://gitlab.com/ark-bitcoin/bark/-/raw/master/bark-rest/openapi.json post /api/v1/wallet/create
openapi: 3.1.0
info:
  title: barkd REST API
  description: >-
    A simple REST API for barkd, a wallet daemon for integrating bitcoin
    payments into your app over HTTP. Supports self-custodial Lightning, Ark,
    and on-chain out of the box.


    barkd is a long-running daemon best suited for always-on or
    high-connectivity environments like nodes, servers, desktops, and
    point-of-sale terminals.


    All endpoints return JSON. Amounts are denominated in satoshis.
  contact:
    name: Second Team
    email: hello@second.tech
  license:
    name: MIT
    identifier: MIT
  version: 0.8.0
servers: []
security:
  - bearer: []
tags:
  - name: boards
    description: Move on-chain bitcoin onto the Ark protocol.
  - name: exits
    description: Move bitcoin back on-chain without server cooperation.
  - name: fees
    description: Estimate fees for wallet operations before executing them.
  - name: history
    description: Inspect and annotate wallet movement history.
  - name: lightning
    description: Create Lightning invoices and track receives.
  - name: message
    description: Sign arbitrary messages with the wallet's keys and verify signed messages.
  - name: onchain
    description: Manage barkd's on-chain bitcoin wallet.
  - name: wallet
    description: Manage Ark balances and VTXOs, send payments via Ark, LN, and on-chain.
  - name: bitcoin
    description: Query bitcoin network data.
  - name: notifications
    description: Receive real-time notifications from barkd.
paths:
  /api/v1/wallet/create:
    post:
      tags:
        - wallet
      summary: Create a wallet
      description: >-
        Creates a new wallet with the specified Ark server and chain source
        configuration. Fails if a wallet already exists. Returns the wallet
        fingerprint on success.
      operationId: create_wallet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/CreateWalletRequest'
        required: true
      responses:
        '200':
          description: Wallet created successfully
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/CreateWalletResponse'
        '500':
          description: Internal server error
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/InternalServerError'
components:
  schemas:
    CreateWalletRequest:
      type: object
      required:
        - network
      properties:
        ark_server:
          type:
            - string
            - 'null'
          description: |-
            The Ark server to use for the wallet.
            Optional when a config.toml already exists in the datadir or when
            the network has a default Ark server (mainnet and signet).
        ark_server_access_token:
          type:
            - string
            - 'null'
          description: |-
            An access token for a private Ark server.

            **Deprecated**: access tokens are no longer enforced by the server;
            this field will be removed in a future release.
          deprecated: true
        birthday_height:
          type:
            - integer
            - 'null'
          format: int32
          description: An optional birthday height to start syncing the wallet from
          minimum: 0
        chain_source:
          oneOf:
            - $ref: '#/components/schemas/ChainSourceConfig'
              description: >-
                The chain source to use for the wallet.

                Optional when a config.toml already exists in the datadir or
                when

                the network has a default chain source (mainnet, signet and

                mutinynet).
            - type: 'null'
        force:
          type: boolean
          description: Proceed even if the datadir contains unexpected files
        gap_limit:
          type:
            - integer
            - 'null'
          format: int32
          description: >-
            How many consecutive unused key indices a VTXO key scan may cross
            before

            it concludes the wallet doesn't own a recovered/imported VTXO.
          maximum: 100000
          minimum: 0
        mnemonic:
          type:
            - string
            - 'null'
          description: The optional mnemonic to use for the wallet
        network:
          $ref: '#/components/schemas/BarkNetwork'
          description: The network to use for the wallet
    CreateWalletResponse:
      type: object
      required:
        - fingerprint
      properties:
        fingerprint:
          type: string
    InternalServerError:
      type: object
      required:
        - message
      properties:
        message:
          type: string
    ChainSourceConfig:
      oneOf:
        - type: object
          description: Use a bitcoind RPC server
          required:
            - bitcoind
          properties:
            bitcoind:
              type: object
              description: Use a bitcoind RPC server
              required:
                - bitcoind
                - bitcoind_auth
              properties:
                bitcoind:
                  type: string
                bitcoind_auth:
                  $ref: '#/components/schemas/BitcoindAuth'
        - type: object
          description: Use an Esplora HTTP server
          required:
            - esplora
          properties:
            esplora:
              type: object
              description: Use an Esplora HTTP server
              required:
                - url
              properties:
                url:
                  type: string
    BarkNetwork:
      type: string
      description: Networks bark can be used on
      enum:
        - mainnet
        - signet
        - mutinynet
        - regtest
    BitcoindAuth:
      oneOf:
        - type: object
          description: Use a cookie file for authentication
          required:
            - cookie
          properties:
            cookie:
              type: object
              description: Use a cookie file for authentication
              required:
                - cookie
              properties:
                cookie:
                  type: string
        - type: object
          description: Use a username and password for authentication
          required:
            - user-pass
          properties:
            user-pass:
              type: object
              description: Use a username and password for authentication
              required:
                - user
                - pass
              properties:
                pass:
                  type: string
                user:
                  type: string
  securitySchemes:
    bearer:
      type: http
      scheme: bearer
      bearerFormat: AuthToken
      description: Base64url-encoded auth token

````

This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
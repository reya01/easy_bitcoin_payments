> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Lightning payments

> Send and receive Lightning payments from your Ark wallet app, with the Ark server as the Lightning gateway and HTLCs keeping payments atomic.

Ark users can transact with anyone on the Lightning Network. An Ark wallet app can pay Lightning invoices and addresses, receive Lightning payments, and interact with any Lightning-enabled wallet app, exchange, or service. The user retains full control of their bitcoin throughout.

On Bark, Lightning connectivity is built directly into the protocol. The Ark server operates its own Lightning node(s) and acts as a gateway between Ark users and the Lightning Network. Users hold VTXOs at rest and spend them to make Lightning payments. This eliminates the usual complexity of Lightning for users and developers: channel management, inbound liquidity, payment routing, and rebalancing are all handled by the Ark server.

Users don't even need an on-chain transaction to start making Lightning payments. A new user can receive VTXOs—from other Ark users or via incoming Lightning payments—and begin transacting over Lightning immediately.

![Diagram showing the Ark server acting as a Lightning gateway between user VTXOs and the Lightning Network](https://mintcdn.com/second-0659a37d/IZIYL7kfKRS394NE/images/lightning-gateway-send.svg?fit=max\&auto=format\&n=IZIYL7kfKRS394NE\&q=85\&s=e8e48c7b66deb03db7a5203b1467dfb2)

The Ark server acts as a Lightning gateway, handling all channel and routing management for users

> [!NOTE]
>
> **Most Bark-based payments will be Lightning payments**
>
> [Ark payments](https://second.tech/docs/learn/payments.md) require both sender and receiver to share the same Ark server. Until individual Ark servers build significant user bases, most payments from Ark wallets will be routed over Lightning. Lightning reaches anyone on the network, regardless of what wallet app or service they use.

## Ark HTLCs

Both outgoing and incoming Lightning payments on Ark use [HTLCs (hash time-locked contracts)](https://bitcoinops.org/en/topics/htlc/) to ensure atomic execution. An HTLC locks bitcoin with two possible outcomes: it can be claimed by revealing a secret (the *preimage*), or it returns to the sender after a timeout. HTLCs are integral to how the Lightning Network routes payments across the network.

## Sending a Lightning payment

When you send a Lightning payment, you spend one of your VTXOs to an HTLC. The Ark server routes the payment to the recipient over the Lightning Network using its own channels. The HTLC serves a similar role to the [forfeit](https://second.tech/docs/learn/forfeits.md) used in [on-chain payments](https://second.tech/docs/learn/payments-on-chain.md): it's an off-chain transaction that the server holds as a safeguard, ensuring you can't reclaim the VTXO after the payment has been delivered.

### The send process

1. **Initiate payment:** You provide a Lightning invoice or address to pay.
2. **Construct HTLC:** Your wallet app and the Ark server cooperate to spend one of your VTXOs into an HTLC—a conditional off-chain output that locks the payment amount until the Lightning payment succeeds or fails.
3. **Route payment:** The Ark server routes the payment over the Lightning Network to the recipient.
4. **Settle:** The server obtains the preimage from the Lightning Network and provides it to you as proof of payment.

![Diagram of the HTLC spend paths and revocation path for sending a Lightning payment](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/lightning-spend.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=fe224e7e81f25816f22039724dbc8c1b)

A Lightning payment's HTLC structure and the revocation path used if a payment fails

### Trust model for Lightning send

When a Lightning payment is delivered successfully, none of the HTLC's spend paths are used. Like a [forfeit](https://second.tech/docs/learn/forfeits.md), the HTLC is held off-chain as a safeguard against malicious exits. The server simply sweeps the round transaction when it expires. The spend paths only come into play when something goes wrong:

| HTLC spend path | Scenario | How it works |
| - | - | - |
| **1. Cooperative revocation** | Payment fails | The server has no preimage and cannot claim your bitcoin. The server co-signs a revocation, returning your bitcoin. |
| **2. Server safeguard** | User attempts a malicious exit | If the user attempts to exit on-chain after the Lightning payment has been successfully delivered, the server can claim the bitcoin by revealing the preimage before the user's timelock expires. |
| **3. User safeguard** | Payment fails and server is uncooperative | If the payment was not delivered and the server stops cooperating, you can reclaim your bitcoin after the timelock expires via an [emergency exit](https://second.tech/docs/learn/exit.md). |

The preimage is what makes Lightning sends atomic: it only exists if the Lightning payment was delivered, and without it the server cannot prevent you from reclaiming your bitcoin.

## Receiving a Lightning payment

On Bark, you can receive Lightning payments from anyone on the Lightning Network. No channels or inbound liquidity are required. The incoming bitcoin is delivered as a VTXO, so bitcoin at rest are always held in VTXOs, not Lightning channel balances.

### The receive process

1. **Generate invoice:** Your wallet app generates a preimage and payment hash on-device, then sends the hash to the Ark server. The server returns a Lightning invoice for the sender to pay.
2. **Incoming payment:** A sender pays the invoice. The payment routes through the Lightning Network and arrives at the Ark server, secured by the preimage. The server sends your wallet app a notification that a payment has been received.
3. **HTLC prep:** Your wallet app comes online and requests an HTLC from the server. The server constructs the HTLC—funded from its VTXO pool and locked to your preimage—then shares it with your wallet app.
4. **HTLC claim:** Your wallet app verifies the HTLC, then reveals the preimage to the server. The server verifies the preimage and completes the payment via an [arkoor transfer](https://second.tech/docs/learn/payments.md). You receive a VTXO.

Steps 3 and 4 are performed back-to-back once your wallet app comes online.

### Trust model for Lightning receive

The receive HTLC has three spend paths, each covering a different scenario:

| HTLC spend path | Scenario | How it works |
| - | - | - |
| **1. Cooperative claim** | Server completes the claim | You reveal the preimage and the server co-signs on the cooperative path, completing an [arkoor payment](https://second.tech/docs/learn/payments.md) from the HTLC to you. You receive a [spend VTXO](https://second.tech/docs/learn/vtxo.md#spend-vtxos). |
| **2. Server safeguard** | User fails to reveal preimage | If you receive the signed HTLC but don't reveal the preimage, and then attempt a malicious exit from the HTLC, this path enables the server to claim back the bitcoin. Even if you can broadcast spend path 3 ("user safeguard") before spend path 2's absolute timelock expires, the preimage will be revealed on-chain, allowing the server to claim the bitcoin from the inbound Lightning payment. |
| **3. User safeguard** | Server fails to complete the claim | If, after you cooperatively reveal the preimage, the server does not complete the arkoor payment from the HTLC, you can initiate an emergency exit via this path. The server safeguard has an absolute timelock which ensures that you have a sufficient window to broadcast your claim first. |

To fund Lightning receive HTLCs, the server maintains a pool of its own VTXOs. When your wallet app requests an HTLC, the server spends one of these VTXOs into the HTLC, signs it with an ephemeral key—a temporary signing key generated for that specific HTLC—then deletes the key before sharing the signed transaction with you.

![Diagram of the receive HTLC spend paths funded from the server's VTXO pool](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/lightning-receive.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=9a686ad3e1b22476d6c7f1a20051cfdb)

Because the HTLC is funded from the server's own bitcoin, you are trusting that the server actually deleted the ephemeral key. If retained, the server could double-spend the HTLC input. To improve the security of a received payment, [refresh](https://second.tech/docs/learn/rounds.md) in any subsequent round to upgrade it to a standard [round VTXO](https://second.tech/docs/learn/vtxo.md#refresh-vtxos).

Lightning receive VTXOs have a much shorter [lifetime](https://second.tech/docs/learn/lifetime.md) than the standard 28 days. The server creates its pool VTXOs with a lifetime of three days (432 blocks) and never funds a receive from a pool VTXO with less than one day (144 blocks) remaining. A Lightning receive VTXO therefore arrives with one to three days until expiry. [This keeps refresh costs low](https://second.tech/docs/learn/rounds.md#round-economics).

> [!NOTE]
>
> **A failed claim currently requires a prompt manual exit**
>
> Bark no longer starts emergency exits automatically when a Lightning receive claim fails. If the server doesn't complete the claim after your wallet app reveals the preimage, Bark retries a few times over the course of a minute or two, then logs a warning and leaves the receive pending. Nothing more happens on its own. Your wallet app has to treat the receive as failed and call `attempt_lightning_receive_exit` in the Bark SDK, either automatically once retries are exhausted or through a control it exposes to you.
>
> Without the exit, a dishonest server could use the revealed preimage to claim the incoming Lightning payment for itself. It could then take the HTLC's bitcoin back through the server safeguard once its timelock expires. What matters is the deadline, not the number of retries. The claim can be retried for as long as the HTLC still has enough time left to complete an exit, roughly 180 blocks, and your wallet app should start the exit before that point. In the worst case, an HTLC granted at the minimum allowed expiry leaves only two blocks (about 20 minutes) of slack after the claim first fails.
>
> Bark previously started exits on Lightning receive failures automatically, but transient claim failures triggered unnecessary exits, so we disabled the behavior. We are working on making claims more robust, and intend to reinstate the automatic exit once that work has proven itself.

### Offline receive, online invoice generation

On Bark, Lightning payments can be received when offline. Your wallet app does not need to be online when the sender pays. Any HTLC from an inbound Lightning payment is secured by your invoice's preimage.

However, you do need to be online for invoice generation, which is an interactive process with the Ark server, and you do need to come online before the Lightning HTLC expires to claim the payment.

## Lightning payment fees

### Sending

Sending a Lightning payment involves several cost components:

* **Liquidity costs:** The Ark server must [deploy bitcoin](https://second.tech/docs/learn/liquidity.md) to route the payment over the Lightning Network. Costs depend on the payment amount and the server's cost of capital.
* **Lightning routing fees:** Standard Lightning Network routing fees for the payment's path through the network.
* **Ark server fees:** The Ark server may charge additional fees to cover operational costs including Lightning node infrastructure.

### Receiving

Receiving Lightning payments is currently free.

See [Second's pricing](https://second.tech/pricing) for current fees.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
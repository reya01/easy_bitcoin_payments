> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Ark fees

> How Ark fees work, why they vary with VTXO lifetime, and the different ways an Ark server can charge for refreshes, Ark, Lightning, and on-chain payments.

Second's Ark server offers transparent, predictable pricing that scales with the size of payments. Holding bitcoin on Ark costs nothing as long as your wallet app refreshes each VTXO during the last two days of its [lifetime](https://second.tech/docs/learn/lifetime.md). For the current fee schedule, see [Second's pricing page](https://second.tech/pricing).

## Each Ark server will have its own fee model

Fee models will differ from server to server. Fees are a matter of policy, not hardcoded into the protocol, and server operators have multiple levers to customize their pricing. One thing is for sure: any Ark server **must** charge fees (in some way or another) to cover their operational costs.

The primary costs an Ark server will incur are:

### Direct costs

* **Liquidity costs:** The opportunity cost of capital deployed for [liquidity operations](https://second.tech/docs/learn/liquidity.md) (e.g., refreshes, Lightning payments, and on-chain payments).
* **On-chain fees:** The transaction fees required to confirm refreshes and on-chain payments on the bitcoin network.
* **Lightning routing fees:** The server covers routing fees for off-Ark Lightning payments.

### Indirect costs

* **Development costs:** The cost of ongoing software development—adding features, optimizing performance, supporting integrations, and fixing bugs.
* **Server operating costs:** The real-world infrastructure costs of running a high-performance, high-uptime server.

## Approaches to charging fees

Broadly, there are two ways Ark servers can approach structuring fees:

* **Direct cost-based fees:** Each operation is priced based on its actual cost to the server. Ark payment fees would be low, while Lightning payments and refreshes would vary based on VTXO age and current on-chain feerates. This ensures the server cannot run at a loss, since every action covers its true cost.

* **Abstracted fees:** The server cross-subsidizes between operations to offer more predictable pricing—for example, charging higher fees for Ark payments to subsidize lower refresh costs, or offering flat rates. This improves the user experience but requires careful design to ensure total costs are covered.

Second adopts a hybrid approach.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
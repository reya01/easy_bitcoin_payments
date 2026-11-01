> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Ark VTXOs

> VTXOs are the building blocks of Ark. Learn about the different types of VTXO (board, refresh, and spend VTXOs) and how transaction trees work.

VTXOs (Virtual Transaction Outputs) are the fundamental building blocks of the Ark protocol. Just as UTXOs are to bitcoin, VTXOs are to Ark—they're how you hold your balance and make payments within the protocol.

When you have bitcoin on Ark, that bitcoin is represented by one or more VTXOs in your wallet. When you want to send bitcoin to someone else on the same Ark server, you spend your VTXOs to create new VTXOs for the recipient—similar to how bitcoin transactions consume UTXOs to create new UTXOs.

Under the hood, a VTXO is a series of pre-signed bitcoin transactions held off-chain by a user that provide the ability to create an on-chain UTXO at a later time, if required. Converting a VTXO into a UTXO can be performed without any third-party interaction, and is known as an "emergency exit". This emergency exit feature is what ensures Ark users are always in control of their bitcoin.

> [!NOTE]
>
> **Offboards vs. exits**
>
> Under normal circumstances, a user that wants to move their bitcoin off Ark will instead perform an offboard or Lightning payment cooperatively with the server, by forfeiting their VTXO (instead of broadcasting it). This is more efficient and cheaper.

## VTXO types overview

There are three types of VTXO in the Ark protocol. You'll receive a different type depending on the operation you're conducting:

* **Board VTXO:** The simplest type, created when moving bitcoin onto Ark. Completely trustless after creation.
* **Refresh VTXO:** Created during periodic server rounds. Standard refresh VTXOs share the same trust model as board VTXOs. More complex transaction trees enable cost sharing among users.
* **Spend VTXO:** Created instantly for payments within Ark. Extends board and refresh VTXOs. Emergency exit requires trusting that previous senders haven't colluded with the server to double-spend.

Each VTXO type uses a different transaction structure, resulting in different trust models and cost profiles.

![Diagram showing all three VTXO types side by side](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/vtxo-types.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=b57bb6130a7cf5efe58ed35b073ea0ac)

Comparison of board VTXO, refresh VTXO, and spend VTXO structures

### VTXO type comparison

| | Board VTXO | Refresh VTXO | Spend VTXO |
| - | - | - | - |
| **Structure** | Root → Leaf (2 transactions) | Root → Branches → Leaf (3+ transactions) | Extended leaf from existing VTXO |
| **Creation timing** | On-demand, needs on-chain confirmations | Periodic rounds only | On-demand, instant |
| **Trust assumptions** | Trustless | Trustless | Assumes sender + server don't collude |
| **Cost** | Full on-chain fee | Partial on-chain fee + liquidity | Marginal |
| **Exit depth** | 2 (root + leaf) | 3+ (root + branches + leaf) | Same as source VTXO + 1 additional |
| **Lifetime** | 28 days | 28 days | Inherited |
| **Typical use case** | Moving bitcoin onto an Ark | Refreshing VTXOs | Sending payments |

> [!NOTE]
>
> **VTXO types are illustrative**
>
> The three VTXO types are a conceptual framework to help explain the structural and trust model differences across VTXOs. They are not defined or distinguished in code—VTXOs are generally treated the same regardless of how they were created.

## Transaction trees

All VTXOs are built on "transaction trees"—structures that enable multiple users to share one large on-chain UTXO while maintaining individual emergency exit rights. A transaction tree consists of pre-signed bitcoin transactions held off-chain, interconnected with strict spending conditions that create a "pseudo covenant".

![Diagram of a simple transaction tree](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/transaction-tree.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=f62c2eeb8da5e2daca44d1e9431a7a34)

A transaction tree with four users, each controlling their individual balance within one on-chain UTXO

Transaction trees contain three transaction types:

* **Root:** The only on-chain transaction, creating the VTXO when confirmed
* **Branch:** Off-chain transactions that split the root's value among multiple users
* **Leaf:** Individual exit transactions that deliver bitcoin to user-controlled addresses

### Transaction spend paths

Each transaction in the tree enforces two mutually exclusive spend paths:

**Spend path 1:** Cooperative multisig between users and server. Root and branch transactions use n-of-n multisigs (MuSig2) between all users sharing that branch plus the server for cooperative operations. Leaf transactions use 2-of-2 multisigs between user and server for forfeits and payments.

**Spend path 2:** Timelocked recovery mechanisms. Root and branch transactions use absolute CLTV timelocks (`<expiry-height> OP_CHECKLOCKTIMEVERIFY`), while leaf transactions use relative CSV timelocks (`<144> OP_CHECKSEQUENCEVERIFY`) for user exits.

![Diagram of a transaction tree output with two spending conditions](https://mintcdn.com/second-0659a37d/92YUTFvK58wCxpgQ/images/spend-paths-root-branch.svg?fit=max\&auto=format\&n=92YUTFvK58wCxpgQ\&q=85\&s=bd6027e27b09dede7fe06e7e66688ca7)

Spending conditions in root and branch outputs

The cooperative path enables normal Ark operations such as spending, refreshing, and offboarding. The timelock paths serve different purposes: root and branch timelocks allow the server to recover bitcoin after expiry to minimize liquidity costs, while leaf timelocks enable users to perform emergency exits from their VTXOs.

> [!NOTE]
>
> **Quad tree structure**
>
> **The diagrams show binary trees for illustration purposes only**. The concepts remain the same, but in practice we use quad trees (each branch transaction splits into four outputs) because they minimize emergency exit costs by reducing the number of transactions a user must broadcast.

## Board VTXOs

Board VTXOs are created when users first move bitcoin onto Ark, representing the simplest VTXO construction.

![Diagram of board VTXO structure](https://mintcdn.com/second-0659a37d/IZIYL7kfKRS394NE/images/board-vtxo.svg?fit=max\&auto=format\&n=IZIYL7kfKRS394NE\&q=85\&s=563daec3b4525d2781b507a80481381d)

Board VTXO structure

**Structure:** The simplest transaction tree with just two transactions. The root spends directly to the user's leaf transaction, skipping the branch layer entirely.

**Trust model:** Board VTXOs are completely trustless from creation until expiry. Users can perform an emergency exit by broadcasting their leaf transaction. Until expiry, the server cannot prevent this unless the user has previously signed a forfeit or spend transaction.

**Exit cost:** Users pay the full on-chain fee to broadcast both root and leaf transactions.

**Lifetime:** By default, board VTXOs have the same [lifetime](https://second.tech/docs/learn/lifetime.md) as refresh VTXOs (28 days on Second's Ark server). However, users can theoretically set a custom lifetime during creation, though this depends on server policy. A custom lifetime could allow users to optimize for expected usage patterns—a shorter lifetime reduces costs for receivers who plan to refresh immediately.

## Refresh VTXOs

Refresh VTXOs are created during Ark server rounds when users must refresh expiring VTXOs or when they want to improve security after receiving spend VTXOs. Rounds occur every hour ([a server-configurable parameter](https://second.tech/docs/learn/rounds.md)).

![Diagram of refresh VTXO structure within a transaction tree](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/refresh-vtxo.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=ecbf8b018cd2cb200b6c9dc2b5cf91b6)

Transaction tree with Alice's refresh VTXO highlighted.

**Structure:** Full transaction trees with multiple branch layers. Users' VTXOs overlap with others in the tree (sharing branches but not leaves), and each user must hold pre-signed transactions for their entire path to exit.

**Trust model:** Standard refresh VTXOs share the same trust model as board VTXOs. Users can perform an emergency exit by broadcasting their portion of the transaction tree. Until expiry, the server cannot prevent exits unless users have signed forfeits or spends.

> [!NOTE]
>
> **Delegated refreshes on mobile devices**
>
> For mobile devices using [delegated refreshes](https://second.tech/docs/learn/rounds.md#delegated-refreshes), refresh VTXOs require trusting that at least one of the designated co-signers behaved honestly during the round. Users who require standard trust model VTXOs must perform a standard, interactive refresh.

**Exit cost:** Users pay individual fees for each of their branch and leaf transactions. In emergency scenarios where many users exit simultaneously, the first user to broadcast each branch transaction saves all other users on that branch from having to broadcast the same transaction. Therefore users exiting earlier subsidize users exiting later.

**Lifetime:** Refresh VTXOs have a [lifetime](https://second.tech/docs/learn/lifetime.md) of 28 days (a server-configurable parameter). Users cannot customize this lifetime—it's set by the server's round policy.

> [!NOTE]
>
> **Users don't hold VTXOs in every round**
>
> One common misconception is that all users of an Ark server participate in *every* round. This is not the case. Users only hold refresh VTXOs in rounds where they personally participated.

## Spend VTXOs

Spend VTXOs are created instantly when users send payments within the same Ark server, using a technique known as arkoor (Ark out-of-round). This mechanism enables immediate transfers without waiting for the next server round. Most users will receive their first Ark balances as spend VTXOs (from friends, customers, or exchanges).

![Diagram of spend VTXO structure within a transaction tree](https://mintcdn.com/second-0659a37d/92YUTFvK58wCxpgQ/images/spend-vtxo.svg?fit=max\&auto=format\&n=92YUTFvK58wCxpgQ\&q=85\&s=90d8f8c1a1e4cf2151d69a109e869192)

Transaction tree with Wendy's spend VTXO highlighted.

**Structure:** Spend VTXOs extend existing transaction trees by adding new leaf transactions. For emergency exit, users must broadcast all transactions in the original tree plus the additional spend transaction, increasing exit length.

**Trust model:** Spend VTXOs follow the [state chain security model](https://bitcoinops.org/en/topics/statechains/). Users can still perform an emergency exit, but they must trust that the sender and server don't collude to double-spend the leaf. As long as either the server or sender acts honestly, payments are secure. [Read more on Ark's payment dynamics](https://second.tech/docs/learn/payments.md).

**Exit cost:** Users must pay transaction fees for their complete exit path: all transactions from the original VTXO tree (all branches and original leaf) plus fees for each additional arkoor transaction in the spending chain. Each spend that extends the chain adds one more transaction fee to the total exit cost. For example, if you receive a spend VTXO that's already been spent twice, you'll pay fees for the original tree plus three arkoor transactions when exiting.

**Lifetime:** Spend VTXOs inherit the [lifetime](https://second.tech/docs/learn/lifetime.md) of their source VTXO. Spending doesn't reset the lifetime, so receivers of "old" VTXOs must refresh sooner but pay lower immediate refresh costs because the server deploys [liquidity](https://second.tech/docs/learn/liquidity.md) for a shorter time period. Receivers of "fresh" VTXOs can wait longer before refreshing but face higher costs if they want to refresh immediately.

Beyond the difference in trust model, spend VTXOs function like any other VTXO type. And the trust assumption only lasts until the VTXO is [refreshed](https://second.tech/docs/learn/rounds.md) (converts it to a standard refresh VTXO) or spent (settled over Ark, Lightning, or on-chain).

### Spend VTXO chains

Spend VTXOs can be chained, with each consecutive spend increasing the exit length by one. A user receiving a spend VTXO in a chain must trust that none of the previous senders colluded with the server to double-spend. However, individual senders cannot unilaterally double-spend—this requires server cooperation.

![Diagram of a chain of spend VTXOs](https://mintcdn.com/second-0659a37d/92YUTFvK58wCxpgQ/images/spend-vtxo-chain.svg?fit=max\&auto=format\&n=92YUTFvK58wCxpgQ\&q=85\&s=c920c269ce11c873e2b1df1367766062)

A spend VTXO (arkoor) chain.

> [!WARNING]
>
> **Spend VTXO chain length considerations**
>
> Long chains increase exit costs and expand the trust surface area. Wallet app developers may want to consider providing users with tools to limit spend VTXO chain length.

### Spend VTXO change

Like spending UTXOs on-chain, users typically spend only a portion of a VTXO, creating two new spend VTXOs:

1. **Payment:** Sent to the recipient
2. **Change:** Returned to the sender

![Diagram of spend VTXO change structure](https://mintcdn.com/second-0659a37d/92YUTFvK58wCxpgQ/images/spend-vtxo-change.svg?fit=max\&auto=format\&n=92YUTFvK58wCxpgQ\&q=85\&s=66db68a9bf7ec1607021307335aaeb08)

How change works when partially spending a VTXO on Ark

Spend VTXO change inherits the trust properties of its source:

* **Change from trustless VTXOs:** Change from board or refresh VTXOs remains trustless since users cannot collude against themselves.
* **Change from self-controlled chains:** Change remains trustless when every transaction in the chain is the user's own (change or self-sends) and the chain originates from the user's own board or refresh VTXO.
* **Third-party chains:** Once other users appear in the spending chain, change inherits the same trust requirements as regular spend VTXOs—users must trust that previous senders in the chain didn't collude with the server.

### Multiple VTXOs for larger payments

Users can send multiple VTXOs simultaneously to complete a single larger payment, creating multiple outputs for the receiver. This approach offers significant advantages over creating multiple small UTXOs on-chain.

There are not the same downsides as doing this on-chain, where the on-chain transaction fees will linearly scale with the number of UTXOs. On the Ark protocol, the cost to send is marginal, and for the receiver, the cost to refresh is relative to the payment size and the amount of liquidity used—not the number of VTXOs being refreshed.

However, there is still one trade-off: sending multiple VTXOs to complete a single payment increases the emergency exit costs for the receiver. If the receiver ever needs to perform an exit, they must broadcast an additional on-chain transaction for each spend VTXO they hold, multiplying their on-chain transaction fees accordingly.

## Transaction tree scalability

Transaction trees scale efficiently with the number of users. The number of transactions a user must broadcast for an emergency exit of a refresh VTXO is one plus the number of branch layers in the tree. The number of branch layers scales logarithmically (`log base 4`) with the number of users. For example:

* 4 users = 1 branch layer (2 total transactions to exit)
* 16 users = 2 branch layers (3 total transactions to exit)
* 64 users = 3 branch layers (4 total transactions to exit)
* 256 users = 4 branch layers (5 total transactions to exit)

This logarithmic scaling means even large numbers of users result in manageable exit transaction counts.

## VTXO fees for users

While each VTXO type has different underlying costs, the fees charged to users may not directly reflect these costs. Servers may choose to abstract or subsidize fees as part of their business model. Fee schedules for Second's Ark server are still a work in progress, but you can read more about considerations on our [fees page](https://second.tech/docs/learn/fees.md).


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
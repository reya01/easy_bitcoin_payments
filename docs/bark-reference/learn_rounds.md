> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Ark rounds

> Users refresh their Ark VTXOs in periodic rounds to extend their lifetime and/or improve security. Learn how rounds work and their coordination process.

Rounds are the heartbeat of an Ark server—periodic events where users can refresh their VTXOs to extend their lifetime.

During a round, multiple users coordinate with the Ark server to forfeit old VTXOs and receive new [refresh VTXOs](https://second.tech/docs/learn/vtxo.md#refresh-vtxos) in return. This is achieved through a collaborative signing process, where, together with the server, users create a transaction tree (a hierarchical structure of pre-signed transactions) and broadcast its root on-chain—the *round transaction*.

![Diagram of a transaction tree produced during a round](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/round-transaction-tree.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=65f0436ecc6228071d13b2d679f52c27)

A simplified example of the kind of transaction tree that is produced during a round

> [!NOTE]
>
> **Payments now occur out-of-round**
>
> Early Ark protocol proposals included payments within rounds, but this is no longer the case. Second's implementation of Ark handles all payments instantly through [arkoor (out-of-round) transactions](https://second.tech/docs/learn/vtxo.md#spend-vtxos). Rounds are now reserved for refreshes only.

## What rounds accomplish

Users participate in rounds for two main reasons:

* **Lifetime management:** Refreshing a VTXO that is nearing the end of its lifetime to extend it and maintain control of their bitcoin.

* **Security upgrade:** Converting [spend VTXOs](https://second.tech/docs/learn/vtxo.md#spend-vtxos) (which have additional trust requirements) into refresh VTXOs with improved security.

## Round process

Refreshing requires the wallet app to participate interactively with other users and the Ark server during the round, signing its own branches in the transaction tree. Once the round confirms, the resulting refresh VTXO can be exited unilaterally without relying on any third party.

| Phase | Description |
| - | - |
| **1. Submit intent** | Wallet app comes online during round and submits VTXOs to refresh |
| **2. Tree construction** | Server constructs transaction tree; wallet app signs exit branches |
| **3. Funding** | Server broadcasts round transaction on-chain |
| **4. Forfeit** | Wallet app signs forfeit to complete refresh |
| **5. Claim** | New VTXO immediately accessible |

This requires the wallet app to be online during scheduled round windows, making it suitable for always-on devices like desktops, servers, and nodes.

## Delegated refreshes

Due to operating system limitations, some devices—particularly iOS devices—cannot reliably wake for scheduled round windows. Delegated signing addresses this by allowing designated co-signers to sign on the user's behalf during the round, with the wallet app claiming the new VTXO when the user next opens the app.

> [!WARNING]
>
> **Co-signer functionality not yet available**
>
> Designated co-signer support has not been built out yet and while it's a priority for us, it is unlikely to be completed until after our mainnet launch. Until co-signers are added to the signing set, delegated refreshes will be dependent on the Ark server's signature only.

### How delegated signing works

Rather than participating interactively during a round, the wallet app submits refresh intents ahead of time—either when the user opens the app or via a lightweight background wake. Designated co-signers then sign on the user's behalf during the round.

| Phase | Delegated refresh process |
| - | - |
| **1. Submit intent** | Wallet app submits refresh intent ahead of time |
| **2. Tree construction** | Server constructs transaction tree; designated co-signers sign on wallet app's behalf |
| **3. Funding** | Server broadcasts round transaction on-chain |
| **4. Forfeit** | Wallet app signs forfeit when user next opens app |
| **5. Claim** | Server or co-signer reveals secret; wallet app claims new VTXO |

![Diagram of the delegated refresh process](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/delegated-refresh.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=40e00777f2f79b210fc5e73fc58ca8bb)

In a delegated refresh, co-signers sign on the user's behalf. The user collects the signed transactions when they next open their wallet.

### Delegated refresh trust model

Delegated refresh VTXOs have reduced security compared to standard refresh VTXOs. Users must trust that at least one designated co-signer behaves honestly—a 1-of-n trust model.

Wallet app providers and other entities act as co-signers, each signing the entire transaction tree. Security is additive: each additional co-signer strengthens the trust model for all users, regardless of which app they use.

Some wallet apps will predefine a fixed co-signer set (presumably including themselves); others will let advanced users specify their own co-signer set. If a required co-signer is unavailable, the delegated refresh fails and alerts the user.

Delegated refresh is entirely opt-in—a practical compromise for mobile OS limitations. With multiple co-signers and a 1-of-n trust model, it offers pretty good security. However, mobile wallet app developers should still consider providing optional alternative paths to standard refreshes, e.g., push notifications that prompt users to open the app before upcoming rounds, or companion desktop/node apps that can refresh on behalf of the mobile wallet.

> [!NOTE]
>
> **Covenants eliminate the trust requirement**
>
> If a covenant proposal (like CTV or TEMPLATEHASH) were adopted on bitcoin, [delegated refreshes would become fully trustless](https://delvingbitcoin.org/t/evolving-the-ark-protocol-using-ctv-and-csfs/1602).

## Round atomicity

Round atomicity is ensured through [hash-locked forfeit transactions](https://second.tech/docs/learn/forfeits.md#how-hash-locks-work-for-rounds). The server generates a preimage for each new VTXO and constructs both the new VTXO and the user's forfeit with a hash-lock condition tied to it. This guarantees that the server cannot claim the old VTXO without also revealing the preimage—which in turn gives the user access to their new VTXO. Either way, the user cannot lose bitcoin.

## Round timing and interval

Second's Ark server conducts rounds every hour (round frequency is a server-configurable parameter).

![Diagram of an example one-hour round frequency](https://mintcdn.com/second-0659a37d/92YUTFvK58wCxpgQ/images/round-interval.svg?fit=max\&auto=format\&n=92YUTFvK58wCxpgQ\&q=85\&s=0dee3863956e07f06d8b883249183e74)

A one-hour round interval results in a round transaction being confirmed roughly every six blocks

### Interval length trade-offs

All things being equal, more frequent rounds will inevitably attract fewer participants per round since users are spread across more opportunities to refresh. Conversely, less frequent rounds lead to more participation demand per round. This fundamental relationship drives several important trade-offs:

| | Shorter round interval | Longer round interval |
| - | - | - |
| **Wait time for refreshes** | Enables users to improve security faster or react more quickly to soon-to-expire VTXOs | Security-conscious users must wait longer for refreshes, and higher risk of VTXO expiries |
| **On-chain fee sharing** | Higher per-user costs due to fewer participants splitting the round transaction fee | Lower per-user costs due to more participants splitting the round transaction fee |
| **Emergency exit** | Fewer transactions to complete exit ([shallower trees](https://second.tech/docs/learn/vtxo.md#transaction-tree-scalability)) | More transactions to complete exit ([deeper trees](https://second.tech/docs/learn/vtxo.md#transaction-tree-scalability)) |

> [!NOTE]
>
> **Flexible round scheduling**
>
> While Second's Ark server uses fixed periodic rounds, servers can technically implement more flexible scheduling:

* **Demand-based rounds:** Queue thresholds for triggering a round
* **Emergency rounds:** On-demand rounds for users with soon-to-expire VTXOs
* **Scheduled maintenance:** Temporary increase in round frequency before planned downtime

## Round economics

Executing rounds involves two major costs for an Ark server:

1. **On-chain transaction fee:** The cost of getting a round transaction confirmed on-chain, which is spread across all users participating in the round—the more users, the lower the cost per user. Despite the underlying transaction tree requiring an n-of-n multisig potentially involving hundreds of users, the Ark protocol uses Taproot and MuSig to make this complex script appear on-chain as a simple single-signature transaction, minimizing the blockchain footprint.

2. **Liquidity deployment costs:** When users refresh their VTXOs, the Ark server must deploy bitcoin ([liquidity](https://second.tech/docs/learn/liquidity.md)) from its wallet to fund the new outputs. The forfeited bitcoin from old VTXOs only becomes available to the server after those VTXOs expire, so there is a cost of capital to cover. This cost is independent of the number of users participating in the round, and is instead dependent on the amount of bitcoin being forfeited and the age of the VTXOs.

> [!NOTE]
>
> Fee schedules for Second's Ark server are still being finalized, and may not directly reflect the server's costs for users' individual operations. You can read more about fee considerations on our [fees page](https://second.tech/docs/learn/fees.md).

### Simultaneous expiry

All VTXOs created in a round share the same expiry time—the absolute timelock on the round transaction and all branch transactions expires simultaneously.

Under normal circumstances, users will forfeit their VTXOs before expiry (through spending, refreshing, or offboarding) to retain control over their bitcoin. This means that when expiry occurs, the server can sweep the entire round's bitcoin balance to its own wallet in a single on-chain transaction, rather than needing to reclaim individual VTXOs separately.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
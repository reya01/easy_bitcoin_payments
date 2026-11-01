> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Ark emergency exits

> Emergency exits are how you move your bitcoin back on-chain from Ark without server cooperation. Learn when exits are needed, how the process works, and costs.

Every VTXO contains pre-signed transactions that let you unilaterally reclaim your bitcoin on the base layer—this is what makes Ark self-custodial.

Under normal conditions, users should always use the more efficient [offboarding](https://second.tech/docs/learn/offboard.md) process. Emergency exits are reserved for situations where the server is unresponsive or refuses to cooperate.

## When are emergency exits required?

Emergency exits serve as an emergency mechanism that ensures users always maintain control over their bitcoin regardless of server behavior. This backup measure is fundamentally what makes Ark self-custodial.

> [!NOTE]
>
> **Emergency exits from different VTXO types**
>
> * **Board and standard refresh VTXOs:** Emergency exits are unilateral. Users can exit without any possibility of prevention by the server or other parties, as long as the exit fully confirms before [the VTXO expires](https://second.tech/docs/learn/lifetime.md#what-happens-when-vtxos-expire).
> * **Spend VTXOs:** While users can independently broadcast emergency exit transactions, the exit can be prevented if the sender and Ark server collude to double-spend. The exit succeeds if it fully confirms before the VTXO expires and either the sender or the server acts honestly. Users can eliminate this trust assumption by refreshing their spend VTXOs in a round.

Under normal operating conditions, users should always follow the standard offboard process when moving bitcoin from Ark back on-chain. Standard offboarding is more efficient and cost-effective for getting bitcoin back on-chain.

Emergency exits should only be considered when the server is unresponsive or refuses to honor offboard requests. These situations typically fall into three categories:

### 1. Malicious behavior

The Ark server may be acting maliciously, attempting to withhold or effectively steal user bitcoin by refusing to process legitimate offboard requests.

### 2. Technical difficulties

The server might experience technical problems preventing normal operation. While these issues can be expected to be temporary, users can still face urgency if they hold VTXOs that are approaching expiry.

### 3. Legal constraints

Legal or regulatory actions may prevent an Ark server from processing certain transactions. In these cases, the server might be legally restricted from honoring offboard requests for specific users or regions.

## The emergency exit process

When a user or their wallet app triggers an emergency exit, they must broadcast a sequence of transactions from their VTXO's transaction tree in the correct order. Each transaction in this sequence must confirm before the next can be broadcast, making this a multi-step process that requires careful coordination.

The exit process follows these steps:

1. **First branch transaction:** The wallet app broadcasts the first branch transaction in their VTXO's path from the root of the transaction tree.
2. **Wait for confirmation:** Once confirmed, the wallet app broadcasts the next branch transaction in the sequence.
3. **Continue down the tree:** This process continues, broadcasting each branch transaction and waiting for confirmation, until reaching the leaf.
4. **Broadcast exit transaction (leaf):** The last transaction in the tree, the *exit transaction*, is broadcast.

Once the exit transaction is broadcast and confirmed, the user's bitcoin is on-chain and will never expire. However, spending this bitcoin still requires the Ark wallet database—losing this data without [a backup of your wallet data](https://second.tech/docs/backups.md) would mean permanently losing access to the exited bitcoin.

To make the bitcoin spendable with standard backup methods (like a seed phrase), users must broadcast a *claim transaction* after the exit transaction's relative timelock expires. This timelock countdown begins when the exit transaction confirms, giving the Ark server and other users time to respond to any malicious exit attempts (trying to exit a VTXO that [has already been forfeited](https://second.tech/docs/learn/forfeits.md#how-forfeits-work) or spent).

![A diagram showing the order of operations for completing an exit and claim](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/emergency-exit-process.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=8132b8d090033fdd6e6f876e4e0964b2)

An example of the complete exit and claim process

### Automated wallet management

Users should never need to manage the emergency exit process manually. Well-designed wallet apps should automate the entire sequence once an exit is triggered. The wallet app will:

* Monitor the blockchain for transaction confirmations
* Automatically broadcast the next transaction in sequence once the previous one confirms
* Track block height to determine when the relative timelock becomes valid
* Execute the final exit transaction to complete the process

> [!NOTE]
>
> **Claims may not be automatic**
>
> Whether the claim process is included as part of the default emergency exit process depends on each wallet app's exit policy and settings.

## Cost of emergency exits

A users performing an emergency exit must pay on-chain transaction fees for each transaction in the exit sequence. The total on-chain fees depend on their refresh VTXO's transaction tree depth, which is determined by the number of VTXOs refreshed in the associated round, plus the length of any additional spend VTXO chain.

> [!WARNING]
>
> **Economic viability of emergency exits**
>
> If the balance held in a VTXO is less than the total on-chain fees required to broadcast all exit transactions, it may not be economically viable to recover those bitcoin. However, as explained in the [early vs. late exit section below](#how-early-exits-reduce-costs-for-later-exiters), other users performing exits from the same round can reduce your exit costs—each transaction another user broadcasts reduces the number of transactions you need to complete by one.

## Partially-completed exits

An exit is not completed until the final exit transaction (leaf) of the transaction tree is broadcast and confirmed on-chain. At any point before that, the bitcoin can still be considered held in the Ark—the VTXO will expire according to its original lifetime and become sweepable by the Ark server.

This gives emergency exits a degree of "cancellability". For instance, a user might begin an exit because an Ark server goes offline for an extended period. If the Ark server comes back online after the user broadcasts the branch transactions that make up their VTXO, they can choose to abort broadcasting the final transaction and keep their bitcoin in the Ark.

Partially completed exits increase the costs for the Ark server to sweep the bitcoin after forfeit and expiry. With the round transaction broken up into multiple smaller transactions, the Ark server must pay multiple on-chain transaction fees to sweep the bitcoin, instead of the typical one.

## How early exits reduce costs for later exiters

When users initiate emergency exits, their actions benefit other users that have participated in the same round(s). This occurs because VTXOs share common branches in the transaction tree. When a user broadcasts a branch transaction as part of their exit, they reduce the number of transactions needed by other users on the same branch by one. In scenarios where many users need to exit, those who exit earlier effectively reduce the exit costs for those who exit later.

## Managing exits during high fee periods

When network fees increase significantly, situations may arise where a VTXO's value becomes smaller than the cost to exit. This raises important considerations for the Ark protocol.

### Server-side

Ark servers are unlikely to monitor or restrict transactions based on economic exit viability:

* Exit viability varies based on multiple factors beyond just VTXO value
* Users have different risk tolerances and fee preferences
* The protocol should remain neutral to economic conditions

### Client-side

Wallet apps should instead implement the appropriate safeguards for users. Options include:

* Warning notifications when a VTXO may cost a significant percentage of the VTXO value to exit
* Exit fee estimation tools to help users make informed decisions
* Options to delay exits to allow other users to exit earlier (thus reducing exit costs)


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
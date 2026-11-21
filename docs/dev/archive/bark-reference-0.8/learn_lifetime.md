> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# VTXO lifetime

> VTXOs have a limited lifetime in the Ark protocol. Understand how VTXO lifetime works and how wallet apps and servers should manage expiries.

All VTXOs have a limited lifetime that determines when the server can claim any remaining bitcoin. Understanding VTXO lifetime is crucial for maintaining control of your bitcoin.

On Second's Ark server, VTXOs created by [boards](https://second.tech/docs/learn/board.md) and [refreshes](https://second.tech/docs/learn/rounds.md) have a 28 day lifetime, while VTXOs from [Lightning receives](https://second.tech/docs/learn/payments-lightning.md#receiving-a-lightning-payment) have a lifetime between 1-3 days. Both are server-configurable parameters.

## What happens when VTXOs expire?

An expired VTXO is not lost, but its owner can only recover the bitcoin with the server's cooperation:

| Operation | After expiry |
| - | - |
| [Refresh](https://second.tech/docs/learn/rounds.md) | Possible |
| [Lightning send](https://second.tech/docs/learn/payments-lightning.md#sending-a-lightning-payment) | Possible |
| [On-chain send](https://second.tech/docs/learn/payments-on-chain.md) or [offboard](https://second.tech/docs/learn/offboard.md) | Possible |
| [Ark send](https://second.tech/docs/learn/payments.md) | Not possible |
| [Emergency exit](https://second.tech/docs/learn/exit.md) | Not possible |

Every VTXO's [transaction tree](https://second.tech/docs/learn/vtxo.md#transaction-trees) has a [timelocked spend path](https://second.tech/docs/learn/vtxo.md#transaction-spend-paths) that unlocks at the expiry height. Once it unlocks, the server sweeps the underlying on-chain bitcoin to its own wallet. The sweep spends the root output that the user's pre-signed exit transactions depend on, rendering them invalid and preventing new Ark payments from being chained onto them.

Refreshes, Lightning sends, and on-chain sends remain possible because they don't rely on extending the transaction tree. The VTXO has already returned to the server's liquidity pool, and the server settles bitcoin over Lightning or on-chain. As normal, any of these operations depends on the server's cooperation. Without emergency exits available, the user has no remedial options if the server does not cooperate. Therefore users should use wallet apps that always keep their VTXOs refreshed.

![Diagram of an Ark server sweeping an expired round to its own wallet](https://mintcdn.com/second-0659a37d/rf2uS0kylw5VUHtT/images/round-sweep-annotated.svg?fit=max\&auto=format\&n=rf2uS0kylw5VUHtT\&q=85\&s=536e310e8d4f14f1ab18f5dba8aa5b84)

Once the VTXOs in a round have expired (simultaneously), the Ark server sweeps the round transaction to its own wallet

## Avoiding expiry

A wallet app has four ways to keep a VTXO from expiring:

* **Refresh:** The default method. The wallet app forfeits the VTXO in a [round](https://second.tech/docs/learn/rounds.md) and receives a new one with a fresh lifetime, automatically and in the background. [Delegated refreshes](https://second.tech/docs/learn/rounds.md#delegated-refreshes) ensure mobile wallet apps can do this even while they're asleep.
* **Spend over Ark:** The recipient receives a [spend VTXO](https://second.tech/docs/learn/vtxo.md#spend-vtxos) that inherits the original expiry, and with it the responsibility to refresh. The server accepts Ark payments from a VTXO right up to its expiry height, so a recipient's wallet app may need to refresh a received VTXO almost immediately.
* **Spend over Lightning or on-chain:** The wallet app forfeits the VTXO and the server delivers the payment [over Lightning](https://second.tech/docs/learn/payments-lightning.md#sending-a-lightning-payment) or [on-chain](https://second.tech/docs/learn/payments-on-chain.md) in an atomic operation. The recipient holds bitcoin outside Ark, with no VTXO to refresh, and the forfeited bitcoin returns to the server's [liquidity](https://second.tech/docs/learn/liquidity.md) once the round is swept.
* **Emergency exit:** If the server stops cooperating, the wallet app broadcasts the VTXO's [exit transactions](https://second.tech/docs/learn/exit.md#the-emergency-exit-process) to claim the bitcoin on-chain. The exit must fully confirm before the expiry height. Reserved for emergencies: it is slow and costs on-chain fees.

In practice, users shouldn't need to worry about VTXO lifetime as long as their wallet app comes online regularly. Well-designed wallet apps should implement automated policies to prevent expiry, such as automatically refreshing VTXOs that approach an expiry threshold and prioritizing older VTXOs for spending.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
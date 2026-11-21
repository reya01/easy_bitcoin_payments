> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Ark offboarding

> Withdraw bitcoin from Ark back to your own on-chain wallet. Offboarding is a cooperative process and more efficient than emergency exits.

Offboarding is the standard, cooperative way to withdraw bitcoin from Ark back to your own on-chain wallet. Users should always prefer offboarding over [emergency exits](https://second.tech/docs/learn/exit.md)—emergency exits should be reserved for situations where the Ark server is unresponsive.

The offboarding mechanism is identical to [on-chain payments](https://second.tech/docs/learn/payments-on-chain.md). On-chain payments are for paying third parties and will use whatever combination of VTXOs (including partial VTXOs) is needed to complete the payment. Offboarding is for withdrawing bitcoin to your own wallet—either a specific whole VTXO or your entire Ark balance.

## Offboard vs. emergency exit

| | Offboard | Emergency exit |
| - | - | - |
| **Server cooperation** | Required | Not required |
| **Cost** | Single on-chain transaction | Multiple on-chain transactions |
| **Speed** | One transaction to confirm | Multiple sequential transactions to confirm |
| **Use case** | Normal withdrawals | Server unresponsive |

If the server is unresponsive, [users can fall back to an emergency exit](https://second.tech/docs/learn/exit.md). While more expensive and slower, emergency exits guarantee users can always access their bitcoin without server cooperation.

## Offboarding fees

Offboarding fees are the same as [on-chain payment fees](https://second.tech/docs/learn/payments-on-chain.md#on-chain-payment-fees).


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
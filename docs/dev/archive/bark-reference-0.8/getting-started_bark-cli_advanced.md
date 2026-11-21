> ## Documentation Index
> Fetch the complete documentation index at: https://second.tech/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Advanced Bark features

> Advanced `bark` features for power users. Learn how to board bitcoin onto Ark and perform emergency exits when the Ark server is unresponsive.

There are two additional `bark` features that we don't expect most users will ever need, but that advanced users might want to try out:

* **Boarding:** Taking on-chain bitcoin off-chain, onto an Ark.
* **Emergency exit:** Taking off-chain bitcoin on-chain, without any cooperation from the Ark server.

## Boarding

We expect most users will receive their first sats over Ark directly, in the same way you received yours from our faucet. But it is definitely possible to move your on-chain bitcoin onto the Ark as well—we call it *boarding*!

Boarding is currently only possible from within Bark's internal on-chain wallet. So for you to try it out, you need to have an on-chain balance.

**1. Check your on-chain balance**

Check your on-chain balance using:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark onchain balance
```

**2. Fund your on-chain wallet**

If you don't have any, [our faucet](https://signet.2nd.dev/) can help you out. Grab an on-chain address from your Bark wallet:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark onchain address
```

Then use the faucet to send some on-chain (signet) sats to this address.

You can run `bark onchain balance` again to double-check they've arrived.

**3. Board the Ark**

Once you have some on-chain sats, you can board the Ark:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
# Board entire on-chain balance
bark board --all

# Board a specified amount
bark board "10000 sat"
```

**4. Verify your VTXOs**

After boarding, you should see a new VTXO holding the sats you boarded:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark vtxos
```

You've successfully taken your on-chain bitcoin off-chain!

## Emergency exit

Finally, let's walk through performing an [emergency exit](https://second.tech/docs/learn/exit.md).

This process requires broadcasting multiple transactions sequentially, so please allocate 1-2 hours to complete this test.

> [!NOTE]
>
> We're currently addressing some known issues with emergency exits that may occur in certain edge cases. [Let us know if you need help](https://community.second.tech).

### What you'll need

To get started, you will need the following:

* At least one VTXO
* At least 15,000 *on-chain* sats per VTXO you're exiting (just to be on the safe side!)
  * Make sure each on-chain UTXO has at least one confirmation. Check this using `bark onchain utxos` or at [mempool.space](https://mempool.space/signet).
* Patience

### Under the hood

Each of your VTXOs consists of a series of off-chain transactions that must be broadcast to retrieve your sats on-chain.

The emergency exit process will work as follows:

* Each of the transactions in your emergency exit will have to be broadcast and confirmed using a [child-pays-for-parent transaction (CPFP)](https://bitcoinops.org/en/topics/cpfp/).
* Each child transaction has to wait an average of 10 minutes for its parent to confirm.
* Once the final transaction of a VTXO's exit is confirmed, you will have to wait for a timelock to expire. The timelock is the VTXO's `exit_delta`: 12 blocks (about two hours) on signet and 144 blocks (about a day) on mainnet.

> [!NOTE]
>
> If you are using your own bitcoin node instead of the Esplora backend, make sure `-txindex` is enabled.

### Performing the exit

**1. Mark VTXOs for exit**

First, you'll need to mark one or more VTXOs that you'd like to exit.

**Mark a single VTXO for exit**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit start --vtxo <your_vtxo_id>
```

**Mark multiple VTXOs for exit**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit start --vtxo <your_first_vtxo_id> --vtxo <your_second_vtxo_id>
```

**Mark all VTXOs in your wallet for exit**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit start --all
```

**2. Initiate your exits**

Run the following command to begin the exit process:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit progress --wait
```

Exits are a multi-step process that requires multiple transactions broadcast in sequence. The `--wait` flag keeps the operation running until the entire VTXO has been exited. We'd recommend going to get a coffee while you wait!

> [!NOTE]
>
> You might see some scary logging. This does not necessarily mean that your exit will fail. Often the process recovers automatically (e.g., retrying after an additional confirmation). If you really want to dig deeper, use the `--verbose` flag to follow everything that happens under the hood.

**3. Claim bitcoin from a completed exit**

Once your exit has been completed—the leaf/exit transaction has been broadcast—your bitcoin is now on-chain and will not expire. However, spending this bitcoin still depends on the data held in your Bark database. If you deleted your Bark wallet data and had no [backup of your wallet data](https://second.tech/docs/backups.md) other than a seed phrase, you would permanently lose access to the exited bitcoin.

To ensure your bitcoin is spendable with only your usual backup setup (e.g., a mnemonic), you'll need to spend the bitcoin to a separate on-chain wallet. This is called a *claim* and requires an additional on-chain transaction.

Don't worry though, Bark makes claiming your exited bitcoin simple:

**Get an on-chain address from your wallet**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark onchain address
```

> [!NOTE]
>
> You don't have to use Bark's on-chain wallet. Any on-chain wallet works.

**Claim emergency exits using your on-chain address**

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit claim <your_onchain_address> --vtxo <your_exited_vtxo_id>
```

When claiming an exit, you can either specify the `--vtxo` flag multiple times or just use the `--all` flag.

### Checking the status of an exit

To check on the status of your exit(s), you can use one of the following commands:

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit list
```

```bash theme={"theme":{"light":"github-light","dark":"github-dark"}}
bark exit status <your_vtxo_id>
```

Both commands support the `--transactions` and `--history` flags which can be extremely helpful when debugging issues.


This documentation is built and hosted on [Mintlify](https://mintlify.com), a developer documentation platform.
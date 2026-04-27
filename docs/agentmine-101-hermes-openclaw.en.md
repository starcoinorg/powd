# AgentMine 101: Start Mining with Hermes and OpenClaw

This guide shows how to start AgentMine mining from OpenClaw in the Hermes experience.

It is written for users who want to know what to do, what each step means, and what result to expect. The concrete actions happen in OpenClaw. Hermes is the broader place where you discover or access AgentMine.

## Who Does What

- `AgentMine` is the mining experience you want to use.
- `Hermes` is the broader product context where you discover or access AgentMine.
- `OpenClaw` is the agent app where you issue natural-language mining commands.
- `powd` is the local component OpenClaw installs in the background to connect your wallet, the mining runtime, and the mining tools.

You do not need to manage low-level mining settings. OpenClaw talks to the local bridge for you.

## Copy-Ready OpenClaw Prompts

Use these prompts inside OpenClaw after the plugin is installed:

```text
install powd
set my wallet to 0x...
show my mining status
start mining
switch to auto mode
pause mining for now
how much reward has this wallet earned?
```

Replace `0x...` with your Starcoin payout wallet address.

## 1. Install The OpenClaw AgentMine Plugin

What you do: install the OpenClaw plugin that lets OpenClaw set up AgentMine mining tools.

Run this in your terminal:

```bash
openclaw plugins install clawhub:@starcoinorg/openclaw-powd
openclaw gateway restart
```

What should happen next: OpenClaw loads the AgentMine mining plugin. After the gateway restart, OpenClaw should understand setup requests for the local mining bridge.

If it fails: make sure OpenClaw is installed and that you can run `openclaw` commands. If the `/powd` command or plugin tools do not appear after installation, restart the OpenClaw gateway again.

## 2. Ask OpenClaw To Install The Local Mining Bridge

What you do: tell OpenClaw to install the local component that connects AgentMine to your machine.

Say this in OpenClaw:

```text
install powd
```

What should happen next: OpenClaw downloads the latest stable `powd` binary, installs it locally, and registers it as the local mining server OpenClaw can call. This setup does not set your wallet and does not start mining.

If it fails: check that your machine can reach GitHub Releases. The OpenClaw plugin currently supports Linux x86_64 and macOS Apple Silicon. If OpenClaw still points to an older path, say `install powd` again so the installer repairs the saved registration.

## 3. Set A Starcoin Payout Wallet

What you do: tell OpenClaw which Starcoin wallet should receive mining payouts.

Say this in OpenClaw:

```text
set my wallet to 0x...
```

What should happen next: OpenClaw saves the wallet locally through the mining bridge. Future mining and reward checks use this wallet unless you change it.

If it fails: confirm the wallet address starts with `0x` and is a Starcoin address. If OpenClaw says mining tools are missing, go back to step 2 and say `install powd`.

## 4. Check Mining Status

What you do: ask OpenClaw what the miner is doing right now.

Say this in OpenClaw:

```text
show my mining status
```

What should happen next: OpenClaw should show the miner state, worker name, requested mode, and recent performance numbers. Before you start mining, it is normal for the miner to show as stopped or not running.

If it fails: if OpenClaw cannot find the mining status tool, restart the OpenClaw gateway. If the status says no wallet is configured, complete step 3 first.

## 5. Start Mining

What you do: explicitly tell OpenClaw to begin mining.

Say this in OpenClaw:

```text
start mining
```

What should happen next: OpenClaw asks the local mining bridge to start the miner with your saved wallet. Mining can use local CPU resources and connect to the pool. OpenClaw may ask for confirmation before it starts.

If it fails: check your wallet is set, then ask for status again. If OpenClaw says the local bridge is missing or unreachable, say `install powd` again and restart the OpenClaw gateway if needed.

## 6. Tune, Pause, Resume, Stop, And Check Rewards

What you do: manage mining after it has been set up.

Say one of these in OpenClaw:

```text
switch to auto mode
pause mining for now
resume mining
stop mining completely
how much reward has this wallet earned?
```

What should happen next:

- `switch to auto mode` lets the local mining bridge choose a safe budget tier automatically.
- `pause mining for now` temporarily pauses mining while keeping your wallet and mode settings.
- `resume mining` continues mining after a pause.
- `stop mining completely` turns mining off until you start or resume it again.
- `how much reward has this wallet earned?` checks reward totals for the configured wallet.

If it fails: use `show my mining status` when you want to know whether mining is running. Use `how much reward has this wallet earned?` when you want payout or earnings totals. If either tool is missing, restart the OpenClaw gateway or repeat `install powd`.

## FAQ

### Does Installing Start Mining Automatically?

No. Installing the plugin and saying `install powd` only prepares the local mining bridge. You still need to set a wallet and say `start mining`.

### Do I Need A Wallet First?

Yes. Mining needs a Starcoin payout wallet before it can start correctly.

### What Is Auto Mode?

Auto mode lets the local mining bridge choose the mining budget automatically. Use it when you want mining to run without choosing `idle`, `light`, `balanced`, or `aggressive` yourself.

### What Is The Difference Between Mining Status And Rewards?

Mining status tells you what is happening locally right now, such as whether the miner is running and which mode is selected.

Rewards tell you what the configured wallet has earned according to the reward service. Rewards are about account totals, not whether the miner is currently running.

### What Platforms Are Supported?

For OpenClaw installation, the plugin currently supports Linux x86_64 and macOS Apple Silicon.

The generic `powd` binary also has a Windows x86_64 release, but this tutorial focuses on the OpenClaw plugin path.

### What If OpenClaw Cannot Find The Mining Tools?

First restart the OpenClaw gateway. If the tools are still missing, say `install powd` again so OpenClaw reinstalls or repairs the local bridge registration. If installation still fails, confirm your platform is supported and that OpenClaw can access GitHub Releases.

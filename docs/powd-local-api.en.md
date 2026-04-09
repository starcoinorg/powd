# `powd` Local API

## Scope

This document describes the currently supported local surfaces around `powd`:

- the public CLI exposed by `powd`
- the MCP tool surface exposed by `powd mcp serve`
- the daemon-private Unix socket JSON-RPC
- the stable status fields that local callers can rely on

For the broader OpenClaw integration rationale, see [powd-openclaw-integration.en.md](powd-openclaw-integration.en.md).

## User model

The public entrypoint is always `powd`.

The persisted user profile is owned by the public `powd` entrypoint, not by the daemon. It contains:

- `wallet_address`
- `worker_name`
- `requested_mode`
- `network`

Supported `network` values:

- `main`
- `halley`

The following values are derived inside the daemon and are not persisted:

- `login = wallet_address.worker_name`
- `pool`
- `pass`
- `consensus_strategy`

## Public CLI

`powd` is the only public human/script entrypoint.

### Wallet commands

- `powd wallet set --wallet-address <addr> [--network main|halley]`
- `powd wallet show`
- `powd wallet reward`

Semantics:

- `wallet set` is the only wallet write command
- on first use it creates a stable `worker_name`
- later calls update `wallet_address`
- `worker_name` stays stable
- `--network` defaults to `halley` on first use; later omission keeps the current network
- if the daemon is already running, `wallet set` reconfigures it immediately
- `wallet reward` is a separate external account query against pool-service
- `wallet reward` uses the persisted `wallet_address + network` and does not depend on the daemon

### Miner commands

- `powd miner status`
- `powd miner start`
- `powd miner stop`
- `powd miner pause`
- `powd miner resume`
- `powd miner set-mode <auto|idle|light|balanced|aggressive>`
- `powd miner watch`

Mode semantics:

- `auto`
  - keeps `requested_mode = auto`
  - lets the daemon compute a finer-grained `effective_budget`
  - does not expose internal governor knobs publicly
- `idle|light|balanced|aggressive`
  - fixed user-facing presets
  - each maps to a fixed `effective_budget`

`pause` and `stop` do not discard `auto`. They place auto into a held state. `resume` and `start` clear that hold.

### Host integration commands

- `powd doctor`
- `powd mcp config`
- `powd mcp config --server-only`
- `powd mcp serve`

Semantics:

- `doctor` checks persisted wallet configuration, daemon reachability, and current runtime state
- `mcp config` prints the standard `mcpServers` JSON snippet for this machine
- `mcp config --server-only` prints just the single MCP server object for `openclaw mcp set`
- `mcp serve` runs the stdio MCP server that OpenClaw launches
- `mcp config` always emits an absolute `powd` path and a stable `env: {}`

## MCP tool surface

`powd mcp serve` exposes these business tools:

- `wallet_set`
- `wallet_show`
- `wallet_reward`
- `miner_status`
- `miner_start`
- `miner_stop`
- `miner_pause`
- `miner_resume`
- `miner_set_mode`

It does not expose:

Reward is deliberately separate from `miner_status`:

- `miner_status` stays pure local daemon state
- `wallet_reward` performs an external HTTP query against pool-service

It does not expose:

- raw `budget.set`
- raw `events.stream`
- `doctor`
- `mcp config`
- daemon-private setup/reconfigure details

CLI and MCP share the same underlying business commands. They are different transports, not different state machines.

## Daemon-private JSON-RPC

`powd` serves a daemon-private Unix socket JSON-RPC. It is used by the public `powd` entrypoint, the dashboard, and diagnostics.

Current methods:

- `daemon.configure`
- `daemon.shutdown`
- `miner.start`
- `miner.stop`
- `miner.pause`
- `miner.resume`
- `miner.set_mode`
- `status.get`
- `status.capabilities`
- `status.methods`
- `events.since`
- `events.stream`

`daemon.configure` is private. It accepts:

- `wallet_address`
- `worker_name`
- `requested_mode`
- `network`

The daemon derives `login`, `pool`, `pass`, and `consensus_strategy` from that profile in memory.

## Startup model

`powd` starts blank. It does not accept public business arguments such as `--login` or `--pool`.

For any `powd` command that needs the daemon:

1. `powd` loads the persisted profile
2. if the daemon is missing, `powd` starts its hidden daemon mode
3. `powd` calls `daemon.configure(profile)`
4. `powd` performs the requested business action

If no persisted profile exists, `wallet set` must be run first.

## Status model

The main read model is `status.get`.

Important fields:

- `state`
- `connected`
- `pool`
- `worker_name`
- `requested_mode`
- `effective_budget`
- `hashrate`
- `hashrate_5m`
- `accepted`
- `accepted_5m`
- `rejected`
- `rejected_5m`
- `submitted`
- `submitted_5m`
- `reject_rate_5m`
- `reconnects`
- `uptime_secs`
- `system_cpu_percent`
- `system_memory_percent`
- `system_cpu_percent_1m`
- `system_memory_percent_1m`
- `auto_state`
- `auto_hold_reason`
- `last_error`

Semantics:

- `requested_mode` is the user's chosen mode
- `effective_budget` is the actual runtime budget now in effect
- when `requested_mode = auto`, `effective_budget` may change over time
- `auto_state` is one of `inactive`, `active`, or `held`
- `auto_hold_reason` is present only when `auto_state = held`

## TUI

`powd miner watch` is the human-facing dashboard.

It shows:

- miner state and connectivity
- `requested_mode`
- `effective_budget`
- `auto_state`
- current and 1-minute system CPU / memory usage
- hashrate and trend counters
- recent events
- the last error

It supports:

- start
- stop
- pause
- resume
- mode changes
- wallet updates

The TUI is only a local presentation and input layer over the same business commands.

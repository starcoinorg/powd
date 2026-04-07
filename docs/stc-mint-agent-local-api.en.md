# `stc-mint-agent` Local API

## 1. Goal

The implemented surface is intentionally narrow:

- `stc-mint-agent` is the only long-running daemon
- `stc-mint-agentctl` is the only front-end
- OpenClaw integrates through `stc-mint-agentctl mcp`
- humans integrate through `stc-mint-agentctl` and `dashboard`
- the user only cares about `wallet_address`
- the default network is `main`

This document describes the current local API only. It does not cover subsidy, growth, agent-side `stratumd`, or remote operations.

## 2. User Model

The user only configures one thing:

- `wallet_address`

The system derives the rest internally:

- create and persist a stable `worker_id`
- compose the final login as `wallet_address.worker_id`
- choose the default mainnet pool
- choose the default `consensus_strategy`
- auto-start `stc-mint-agent` when needed

The user may change the payout address at any time:

- only `wallet_address` changes
- `worker_id` stays stable
- if the daemon is running, the new login takes effect immediately through a managed restart

Local persistent state stores only:

- `wallet_address`
- `worker_id`

There is no user-facing miner config file.

## 3. Component Boundaries

### 3.1 `stc-mint-agent`

`stc-mint-agent` owns:

- the miner core
- the JSON-RPC local API
- the lifecycle state machine
- status, trend metrics, and the event buffer

It does not own:

- OpenClaw scheduling policy
- user interaction
- TUI
- MCP

### 3.2 `stc-mint-agentctl`

`stc-mint-agentctl` is the unified front-end with three entry forms:

- normal CLI
- the `mcp` subcommand
- the `dashboard` subcommand

It talks to `stc-mint-agent` over the local Unix socket and auto-starts the daemon when needed.

### 3.3 OpenClaw

OpenClaw does not talk to the miner directly and does not build raw CLI strings.

It registers:

- `stc-mint-agentctl mcp`

Then it calls the exposed MCP tools.

The scheduling loop also stays in OpenClaw:

- read `status` and `events_since`
- combine them with system CPU, memory, user activity, and power state
- decide when to `set_mode`, `pause`, `resume`, `start`, and `stop`

## 4. MCP Tool Surface

`stc-mint-agentctl mcp` exposes only the safe tool surface:

- `setup`
- `set_wallet`
- `status`
- `capabilities`
- `methods`
- `start`
- `stop`
- `pause`
- `resume`
- `set_mode`
- `events_since`

It does not expose:

- raw `budget.set`
- raw `events.stream`
- pool, password, worker, or network selection

### 4.1 `setup`

Input:

- `wallet_address`

Effect:

- persist the wallet address
- create a stable `worker_id` if one does not exist yet
- return the derived local config summary

### 4.2 `set_wallet`

Input:

- `wallet_address`

Effect:

- update the wallet address
- keep the same `worker_id`
- if the daemon is running, restart it in a managed way so the new login takes effect immediately

### 4.3 `set_mode`

Only preset modes are allowed:

- `conservative`
- `idle`
- `balanced`
- `aggressive`

The upper layer is not allowed to send raw budget values.

## 5. CLI and TUI

### 5.1 CLI

The current human/script entrypoints are:

- `setup --wallet-address ...`
- `set-wallet --wallet-address ...`
- `status`
- `start`
- `stop`
- `pause`
- `resume`
- `set-mode <mode>`
- `doctor`
- `mcp-config`

`doctor` checks:

- whether the wallet is configured
- whether `worker_id` exists
- whether the daemon is reachable
- current miner state and the last error

`mcp-config` prints an MCP registration snippet that can be pasted into OpenClaw.

### 5.2 Dashboard

`stc-mint-agentctl dashboard` is the local human-facing TUI.

v1 shows:

- current state
- connection state
- hashrate / `hashrate_5m`
- accepted / rejected / submitted
- `reject_rate_5m`
- current budget
- recent events
- the last error

v1 operations:

- `s` start
- `x` stop
- `p` pause
- `r` resume
- `1` conservative
- `2` idle
- `3` balanced
- `4` aggressive
- `w` update wallet address
- `q` quit

## 6. Daemon Auto-Start

Before any MCP, CLI, or dashboard operation that needs a daemon, the front-end probes the local socket.

If the daemon is missing:

1. check whether `wallet_address` and `worker_id` are ready
2. require `setup` if they are missing
3. otherwise start `stc-mint-agent` automatically

The startup arguments are derived internally:

- default network = `main`
- default pool = the mainnet pool
- default `consensus_strategy` = the mainnet default algorithm
- login = `wallet_address.worker_id`

These details stay hidden from the user and from OpenClaw.

## 7. Read Path

### 7.1 Status

The primary read methods are:

- `status.get`
- `status.capabilities`
- `status.methods`

`status.get` includes at least:

- `state`
- `connected`
- `pool`
- `worker_name`
- `current_mode`
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
- `current_budget`
- `last_error`

`current_mode` has fixed semantics:

- preset changes through `set_mode` return the active mode
- raw `set-budget` switches into a custom budget and returns `null`

### 7.2 Events

The main event method for OpenClaw is:

- `events.since`

It returns:

- `next_seq`
- `events`

OpenClaw polls it in a request-response loop instead of relying on `events.stream`.

`events.stream` remains available for human debugging and long-lived CLI listeners only.

## 8. Current Constraints

The current shape deliberately keeps these constraints:

- the user flow is mainnet-only by default
- `halley` is not exposed in the normal user path
- OpenClaw does not set raw budget values
- automatic scheduling stays out of the miner and out of the MCP bridge
- `stc-mint-agent` is the only long-running process; there is no second adapter daemon

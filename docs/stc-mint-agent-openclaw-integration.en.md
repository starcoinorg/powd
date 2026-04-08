# `stc-mint-agent` and OpenClaw Integration

## Purpose

This document fixes the supported third-party integration boundary for OpenClaw:

- where the main scheduling loop lives
- how OpenClaw integrates without source patches
- how the package is installed and handed to users
- why the system is organized that way

It is the canonical integration document. The concrete command and API reference stays in `docs/stc-mint-agent-local-api.en.md`.

## Final organization

The supported shape has three responsibilities:

- `stc-mint-agent`
  - the only daemon
  - owns the active miner runtime, local API, event history, and internal auto loop
- `stc-mint-agentctl`
  - the only public front-end
  - owns persisted user profile, CLI, TUI, and the MCP bridge
- OpenClaw
  - registers the MCP bridge
  - calls MCP tools
  - provides higher-level UX

This deliberately rejects:

- patching OpenClaw source for basic integration
- putting the main scheduling loop into a skill prompt
- putting the main scheduling loop into OpenClaw plugin code
- adding a second adapter daemon beside `stc-mint-agent`

## Why the loop lives in the daemon

The main loop belongs in `stc-mint-agent` because the daemon already owns the long-lived runtime concerns:

- the active miner runtime
- reconnect and runtime transitions
- event buffering
- trend metrics
- the effective runtime budget

That loop is deterministic code, not an LLM prompt loop.

`stc-mint-agentctl` owns user intent and bootstrapping, but the daemon owns the actual long-lived miner execution. That makes the policy durable even when OpenClaw is closed.

## Adaptation path

The formal OpenClaw entrypoint is:

- `stc-mint-agentctl integrate mcp`

This command runs a stdio MCP server.

OpenClaw only needs to register that command. It does not need to know the daemon's private socket protocol.

The MCP bridge exposes only the public business tools:

- `wallet_set`
- `wallet_show`
- `miner_status`
- `miner_start`
- `miner_stop`
- `miner_pause`
- `miner_resume`
- `miner_set_mode`

It intentionally hides:

- `daemon.configure`
- raw `budget.set`
- raw event streams
- pool / pass / worker / strategy details
- install-only or diagnostic-only commands

## User-facing install path

The user-facing package contains:

- `stc-mint-miner`
- `stc-mint-agent`
- `stc-mint-agentctl`

The normal install path is:

1. install the package
2. configure the wallet once:
   - `stc-mint-agentctl wallet set --wallet-address <addr> [--network main|halley]`
3. if OpenClaw is used, print the MCP snippet:
   - `stc-mint-agentctl integrate mcp-config`
4. register that MCP command in OpenClaw
5. operate through:
   - OpenClaw tools
   - or `stc-mint-agentctl miner watch`

Defaults:

- `network = main`
- `worker_id` is generated automatically on first wallet set
- `requested_mode = auto` on first wallet set

The user does not need to manage:

- `login`
- `pool`
- `pass`
- `consensus_strategy`

## Wallet changes

Changing the payout wallet is part of the normal flow.

When the user runs `wallet set` again:

- `wallet_address` is updated
- `worker_id` stays stable
- `network` stays unchanged unless `--network` is explicitly provided
- if the daemon is already running, `ctl` reconfigures it immediately through the private API
- the daemon preserves runtime intent across that reconfiguration

## Why this boundary is the best fit

This organization gives a clean split:

- `ctl` owns user intent and persisted profile
- the daemon owns runtime execution and automatic budgeting
- OpenClaw uses MCP as the supported host boundary

That keeps third-party integration realistic:

- no OpenClaw source dependency
- no second long-lived adapter process
- no duplicated business configuration in both `ctl` and daemon startup flags

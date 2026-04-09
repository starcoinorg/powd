# `powd` and OpenClaw Integration

## Purpose

This document fixes the supported third-party integration boundary for OpenClaw:

- where the main scheduling loop lives
- how OpenClaw integrates without source patches
- how the package is installed and handed to users
- why the system is organized that way

It is the canonical integration document. The concrete command and API reference stays in [powd-local-api.en.md](powd-local-api.en.md).

## Final organization

The supported shape has three responsibilities:

- `powd`
  - the only public front-end
  - owns persisted user profile, CLI, TUI, and the MCP bridge
  - self-bootstraps its hidden daemon mode when runtime work is needed
- hidden `powd` daemon mode
  - owns the active miner runtime, local API, event history, and internal auto loop
- OpenClaw
  - registers the MCP bridge
  - calls MCP tools
  - provides higher-level UX

This deliberately rejects:

- patching OpenClaw source for basic integration
- putting the main scheduling loop into a skill prompt
- putting the main scheduling loop into OpenClaw plugin code
- adding a second adapter daemon beside `powd`

## Why the loop lives in the daemon

The main loop belongs in `powd` because the daemon already owns the long-lived runtime concerns:

- the active miner runtime
- reconnect and runtime transitions
- event buffering
- trend metrics
- the effective runtime budget

That loop is deterministic code, not an LLM prompt loop.

The public `powd` entrypoint owns user intent and bootstrapping, but the hidden daemon mode owns the actual long-lived miner execution. That makes the policy durable even when OpenClaw is closed.

## Adaptation path

The formal `powd` host entrypoints are:

- `powd mcp serve`
- `powd mcp config`

`powd mcp serve` runs the stdio MCP server.

`powd mcp config` prints a standard local MCP config snippet with:

- an absolute `powd` path
- `args = ["mcp", "serve"]`
- `env = {}`

OpenClaw only needs to register that command. It does not need to know the daemon's private socket protocol.

For OpenClaw-managed saved config, the supported registration flow is:

1. `powd mcp config --server-only`
2. `openclaw mcp set powd '<json>'`
3. `openclaw mcp show powd --json`

OpenClaw's `mcp set/show/list/unset` commands only manage saved config. They do not prove that the target MCP server is reachable right now.

The MCP bridge exposes only the public business tools:

- `wallet_set`
- `wallet_show`
- `wallet_reward`
- `miner_status`
- `miner_start`
- `miner_stop`
- `miner_pause`
- `miner_resume`
- `miner_set_mode`

Each tool definition is expected to carry host-facing routing hints:

- explicit titles and longer descriptions for natural-language routing
- JSON Schema `examples` for argument-bearing tools
- read-only vs mutating annotations for host policy
- destructive hints on `wallet_set` and `miner_stop`, so hosts can require confirmation before changing payout identity or halting live mining

It intentionally keeps account rewards separate from miner runtime state:

- `wallet_reward` is an external pool-service account query
- `miner_status` remains local daemon state only

It intentionally hides:

- `daemon.configure`
- raw `budget.set`
- raw event streams
- pool / pass / worker / strategy details
- install-only or diagnostic-only commands

## User-facing install path

The OpenClaw-facing package contains:

- `powd`

`powd-miner` remains a low-level debug binary. It is not part of the normal OpenClaw install path.

The normal install path is:

1. install the package
2. configure the wallet once:
   - `powd wallet set --wallet-address <addr> [--network main|halley]`
3. if OpenClaw is used, print the MCP snippet:
   - `powd mcp config`
4. register that MCP command in OpenClaw
5. operate through:
   - OpenClaw tools
   - or `powd miner watch`

Defaults:

- `network = halley`
- `worker_name` is generated automatically on first wallet set
- `requested_mode = auto` on first wallet set

The user does not need to manage:

- `login`
- `pool`
- `pass`
- `consensus_strategy`

## Repo-local clean verification

For development and verification inside this repository, there is also a pinned OpenClaw shell:

1. `nix develop .#openclaw`
2. `scripts/mcp-smoke.sh`

That shell:

- fetches a pinned OpenClaw GitHub source tarball
- builds OpenClaw locally with pinned `node` and `pnpm`
- isolates `OPENCLAW_HOME` under `.tmp/openclaw`

It is only a repo-local verification path. It is not the public user install story for `powd`.

## Wallet changes

Changing the payout wallet is part of the normal flow.

When the user runs `wallet set` again:

- `wallet_address` is updated
- `worker_name` stays stable
- `network` stays unchanged unless `--network` is explicitly provided
- if the daemon is already running, `powd` reconfigures it immediately through the private API
- the daemon preserves runtime intent across that reconfiguration

## Why this boundary is the best fit

This organization gives a clean split:

- the public `powd` entrypoint owns user intent and persisted profile
- the hidden daemon mode owns runtime execution and automatic budgeting
- OpenClaw uses MCP as the supported host boundary

That keeps third-party integration realistic:

- no OpenClaw source dependency
- no second long-lived adapter process
- no duplicated business configuration in both the public entrypoint and daemon startup flags

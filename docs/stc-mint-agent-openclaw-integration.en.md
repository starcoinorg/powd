# `stc-mint-agent` and OpenClaw Integration

## 1. Purpose

This document answers one set of organizational questions:

- where the dynamic scheduling loop belongs
- how OpenClaw should integrate
- how installation should be packaged for users
- why the system should be organized that way

This is a **target organization document**, not the detailed local API reference. The concrete interface reference stays in:

- `docs/stc-mint-agent-local-api.en.md`

## 2. Final Position

The best-practice shape is four fixed decisions:

- `stc-mint-agent` is the only daemon that owns long-lived business state
- the dynamic scheduling loop lives in the daemon as a `governor`
- OpenClaw integrates through `stc-mint-agentctl mcp` as a **stdio MCP** entrypoint
- `skill` / `plugin` are only discovery, install, and UX layers; they do not own the business loop

These options are explicitly rejected:

- no requirement to patch OpenClaw source
- no main loop inside OpenClaw internal code
- no main loop inside a skill prompt
- no second adapter daemon

## 3. Component Organization

### 3.1 `stc-mint-agent`

`stc-mint-agent` owns:

- the miner core
- `wallet_address`
- the stable `worker_id`
- the derived login `wallet_address.worker_id`
- runtime state, trend metrics, and the event buffer
- the `governor` scheduling state

`governor` is the daemon-local deterministic scheduling subsystem. It is responsible for:

- periodic sampling of system load and miner health
- deciding between `conservative / idle / balanced / aggressive`
- maintaining raise/lower cooldown and freeze state
- suspending automatic scheduling after manual overrides

### 3.2 `stc-mint-agentctl`

`stc-mint-agentctl` is the unified front-end and only owns three forms:

- human CLI
- the `dashboard` TUI
- the `mcp` bridge

It does not own a second copy of business state and it does not run a second scheduling loop.

### 3.3 OpenClaw

OpenClaw, as the host, only owns:

- registering `stc-mint-agentctl mcp`
- calling MCP tools
- showing state
- providing manual overrides
- improving discovery and UX through skill / plugin

OpenClaw does not need source patches and it does not own the miner's long-lived business state.

## 4. Why the loop belongs in the daemon

The main loop belongs in `stc-mint-agent`, not in OpenClaw, for four reasons:

- third-party integrations cannot assume long-term control over OpenClaw source
- `wallet_address`, `worker_id`, login, pool connection, and the event buffer already live in the daemon; the loop belongs next to that state
- `skill` is a prompt layer and `plugin` is an integration layer; neither is a good home for a persistent business loop with cooldown and freeze logic
- the miner should keep a stable local policy even when OpenClaw is closed

OpenClaw still matters, but its role narrows to:

- local MCP client
- user entrypoint
- manual override and visibility layer

## 5. Installation and Distribution

The v1 user-facing distribution stays fixed to three binaries:

- `stc-mint-miner`
- `stc-mint-agent`
- `stc-mint-agentctl`

The user path is wallet-first:

1. install the package
2. run once:
   - `stc-mint-agentctl setup --wallet-address <addr>`
3. if OpenClaw is used, run:
   - `stc-mint-agentctl mcp-config`
4. register the generated MCP config in OpenClaw
5. use the system through:
   - OpenClaw MCP tools
   - or `stc-mint-agentctl dashboard`

The user only deals with:

- `wallet_address`

The system derives the rest automatically:

- default network = `main`
- default pool = the mainnet pool
- default algorithm = the mainnet default algorithm
- auto-generated stable `worker_id`
- derived login
- daemon auto-start when needed

When the payout address changes:

- only `wallet_address` changes
- `worker_id` stays stable
- the new login takes effect immediately through hot swap or a managed restart

## 6. OpenClaw Adaptation Path

The formal OpenClaw integration surface is fixed to:

- `stc-mint-agentctl mcp`

This is a stdio MCP server. OpenClaw only needs to register it; it does not need to understand the miner's internal protocol.

The MCP surface exposes only safe tools:

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
- later governor-specific tools should stay governance-oriented and should not expose raw `budget.set`

`skill` and `plugin` have fixed optional roles:

- `skill`: teach the model when to use which MCP tools
- `plugin`: help users register MCP and provide better UI or install flow

Neither should own the miner's primary state or run the business loop.

## 7. Final User Experience

The final product mental model must collapse to two points:

- give me a `wallet_address`
- default to main; I do not care about the rest

Human operators mainly use:

- `stc-mint-agentctl dashboard`
- a small CLI surface

OpenClaw users mainly use:

- the registered MCP tools

Both paths share the same daemon, the same state, and the same scheduling rules.

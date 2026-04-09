# `powd`

`powd` is an agent-facing local runtime that turns spare CPU into sustainable Starcoin budget.

The product is not "a miner with some shell commands on top". The intended shape is:

- an agent uses `powd` as a local capability
- MCP is the supported host boundary
- natural language sits above that capability surface
- `powd` is the public control plane, while daemon mode stays internal

## Agent-first model

`powd` exists so an agent can reason about local compute budget in the same way it reasons about files, terminals, or web tools.

An agent can use `powd` to:

- attach and persist a payout wallet
- inspect miner runtime state and reward state
- start, stop, pause, or resume mining
- shift between `auto`, `conservative`, `idle`, `balanced`, and `aggressive`
- make those actions available through natural language on top of a strict MCP tool surface

That means the core UX is not "memorize commands". The core UX is "let an agent understand intent, inspect the current state, and operate the runtime safely".

## System shape

The supported split is:

- `powd`
  - the only public front-end
  - owns persisted user profile, CLI/TUI, and the MCP bridge
- hidden `powd` daemon mode
  - owns miner runtime, local state, event history, and automatic budgeting
- OpenClaw or another MCP host
  - discovers `powd` tools
  - routes natural language into those tools
  - provides higher-level agent UX

This keeps the long-lived runtime in deterministic code while letting the host layer provide agent behavior and natural-language interaction.

## Integration boundary

`powd` is designed to be consumed by hosts, not patched into them.

The MCP surface intentionally exposes a small business tool set around wallet identity, runtime control, reward lookup, and mining mode. That is the stable layer an agent can learn and automate against. The daemon's private socket protocol, raw budget controls, and internal event plumbing stay behind that boundary.

For OpenClaw, the integration model is standard local MCP over `stdio`. `powd` provides the registration shape, launches the bridge, and self-bootstraps its hidden daemon mode when runtime work is needed. The host only needs to register `powd` and call tools.

Releases after the single-entrypoint refactor no longer ship `powctl`. Existing scripts or host registrations that still call `powctl` need to be updated to `powd`.

## Why this matters

If `powd` were only a CLI, natural language would just be a thin translation layer over shell commands.

The point of `powd` is narrower and more useful:

- keep the execution model local and deterministic
- expose a host-friendly capability surface
- let agents operate that surface without inventing a second runtime

In other words, `powd` is the local compute-budget runtime, and the host is where agent behavior lives.

## Docs

- [Docs index](docs/README.md)
- [OpenClaw integration](docs/powd-openclaw-integration.en.md)
- [Local API](docs/powd-local-api.en.md)
- [Identity and minimal protocol](docs/powd-identity.en.md)

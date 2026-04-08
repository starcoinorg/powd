# `powd`

`powd` is a wallet-first CPU mining daemon for Starcoin.

It exposes three binaries:

- `powd`
  - internal daemon
- `powctl`
  - public CLI, TUI, and MCP bridge
- `powd-miner`
  - raw low-level miner/debug entrypoint

## User model

The public entrypoint is `powctl`.

Typical flow:

```bash
powctl wallet set --wallet-address <address>
powctl miner start
powctl miner watch
```

OpenClaw integration goes through:

```bash
powctl integrate mcp
```

## Docs

- [Docs index](docs/README.md)
- [Local API](docs/powd-local-api.en.md)
- [OpenClaw integration](docs/powd-openclaw-integration.en.md)
- [Identity and minimal protocol](docs/powd-identity.en.md)

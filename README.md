# `powd`

`powd` gives MCP-capable agents a local mining capability they can use through natural language.

`powd` works with any host that can launch a local stdio MCP server. Today that includes OpenClaw, Codex, and Claude Code.

## What You Can Do

- connect a Starcoin payout wallet
- check mining status
- check wallet rewards
- start, pause, resume, or stop mining
- switch between `auto`, `idle`, `light`, `balanced`, and `aggressive`

## Install `powd`

For Codex, Claude Code, or any other generic MCP host, install the `powd` binary from GitHub Releases first.

- Download the latest stable `powd` release for your platform from GitHub Releases.
- Supported platforms today:
  - Linux x86_64
  - macOS Apple Silicon
  - Windows x86_64
- Place the `powd` binary somewhere on your `PATH`, then verify:

```bash
powd --help
```

## Choose Your Host

### OpenClaw

Install the OpenClaw plugin:

```bash
openclaw plugins install clawhub:@starcoinorg/openclaw-powd
openclaw gateway restart
```

Then in OpenClaw, say:

```text
install powd
```

OpenClaw will download the latest stable `powd` binary, install it locally, and register it as an MCP server.

### Codex

After installing the `powd` binary locally, add it as an MCP server:

```bash
codex mcp add powd -- /absolute/path/to/powd mcp serve
```

### Claude Code

After installing the `powd` binary locally, add it as an MCP server:

```bash
claude mcp add --transport stdio powd -- /absolute/path/to/powd mcp serve
```

### Other MCP Hosts

If your host can launch a local stdio MCP server, point it at:

```bash
powd mcp serve
```

If your host accepts a `command` / `args` / `env` configuration object, you can use:

```bash
powd mcp config
```

## What To Say After Setup

- `Set my wallet to 0x...`
- `Show my mining status`
- `How much reward has this wallet earned?`
- `Start mining`
- `Pause mining for now`
- `Switch to balanced mode`
- `Set mining mode to auto`

## What To Expect

- wallet settings are stored locally on your machine
- your agent host talks to `powd` through MCP
- `powd` starts its own local runtime when it needs to do work
- OpenClaw installation does not automatically start mining

## Troubleshooting

- `The /powd command does not appear`
  Restart the OpenClaw gateway after installing the plugin.

- `Your host cannot find powd`
  Make sure the `powd` binary is installed locally and available at the exact path you registered.

- `install powd fails in OpenClaw`
  The OpenClaw plugin currently supports Linux x86_64 and macOS Apple Silicon, and it needs access to GitHub Releases.

- `OpenClaw still points to an older powd path`
  Ask OpenClaw to `install powd` again. The installer repairs the saved MCP registration.

- `Mining commands fail because no wallet is configured`
  Ask your agent to set your wallet first.

## Learn More

- [Docs index](docs/README.md)
- [OpenClaw integration](docs/powd-openclaw-integration.en.md)
- [Local API](docs/powd-local-api.en.md)
- [Identity and minimal protocol](docs/powd-identity.en.md)

# `powd`

`powd` gives OpenClaw a local mining capability that an agent can use through natural language.

You do not need to install or register the binary by hand. The normal path is: install the OpenClaw plugin, then ask OpenClaw to install `powd` for you.

## What You Can Do

- connect a Starcoin payout wallet
- check mining status
- check wallet rewards
- start, pause, resume, or stop mining
- switch between `auto`, `conservative`, `idle`, `balanced`, and `aggressive`

## Quick Start

### 1. Install the OpenClaw plugin

Download the `openclaw-powd-<version>.tgz` plugin archive from the release page, then install it into OpenClaw:

```bash
openclaw plugins install ./openclaw-powd-<version>.tgz
openclaw gateway restart
```

This adds the `powd` installer inside OpenClaw. It does not start mining.

### 2. In OpenClaw, say `install powd`

Once the plugin is loaded, tell OpenClaw:

```text
install powd
```

OpenClaw will:

- download the latest stable `powd` binary from GitHub Releases
- install it locally
- register it as an MCP server

After that, you can do the rest through chat.

If you need a specific release instead, ask OpenClaw to install that version explicitly, for example `install powd 1.0.0-rc.1`.

## What To Say In OpenClaw

- `Set my wallet to 0x...`
- `Show my mining status`
- `How much reward has this wallet earned?`
- `Start mining`
- `Pause mining for now`
- `Switch to balanced mode`
- `Set mining mode to auto`

## What To Expect

- wallet settings are stored locally on your machine
- OpenClaw talks to `powd` through MCP
- `powd` starts its own local runtime when it needs to do work
- installing `powd` does not automatically start mining

## Troubleshooting

- `The /powd command does not appear`
  Restart the OpenClaw gateway after installing the plugin.

- `install powd fails`
  `powd` plugin v1 currently supports Linux x86_64 and needs access to GitHub Releases.

- `OpenClaw still points to an older powd path`
  Ask OpenClaw to `install powd` again. The installer repairs the saved MCP registration.

- `Mining commands fail because no wallet is configured`
  Ask OpenClaw to set your wallet first.

## Learn More

- [Docs index](docs/README.md)
- [OpenClaw integration](docs/powd-openclaw-integration.en.md)
- [Local API](docs/powd-local-api.en.md)
- [Identity and minimal protocol](docs/powd-identity.en.md)

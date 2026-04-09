# `@starcoinorg/openclaw-powd`

`powd` gives OpenClaw a local mining capability that an agent can use through natural language.

## How To Use In OpenClaw

1. Install the plugin in OpenClaw:

   ```bash
   openclaw plugins install clawhub:@starcoinorg/openclaw-powd
   openclaw gateway restart
   ```

2. In OpenClaw, say `install powd`.
3. Then keep going in chat, for example:
   - `set my wallet to 0x...`
   - `show my mining status`
   - `start mining`

## What it does

- adds `powd_install` and `powd_setup_status` tools for agents
- adds `/powd install` and `/powd status` commands
- adds `openclaw powd install` and `openclaw powd status` CLI subcommands
- downloads the latest stable `powd` release binary on demand
- writes `mcp.servers.powd` in OpenClaw config

It does **not** set a wallet or start mining automatically.

## Local Packaging

Create a local plugin archive with:

```bash
npm pack
```

Install it into OpenClaw with:

```bash
openclaw plugins install ./openclaw-powd-<version>.tgz
openclaw gateway restart
```

Then, inside OpenClaw, say:

```text
install powd
```

To pin a specific release instead of the latest stable one, say:

```text
install powd 1.0.0-rc.1
```

## Test override

For local smoke tests, the plugin respects:

- `POWD_PLUGIN_RELEASE_BASE_URL`
- `POWD_PLUGIN_RELEASE_API_BASE_URL`

Set it to a base URL shaped like:

```text
http://127.0.0.1:<port>/releases/download
```

The plugin will append `/v<version>/<asset>` automatically.

For the latest stable lookup endpoint, use a base URL shaped like:

```text
http://127.0.0.1:<port>/api/releases
```

The plugin will request `/latest` from that API base.

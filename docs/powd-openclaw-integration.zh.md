# `powd` 与 OpenClaw 集成

## 目的

这份文档固定第三方接入 OpenClaw 时的支持边界：

- 主调度 loop 放哪里
- 在不改 OpenClaw 源码的前提下怎么接
- 包怎么安装和交付给用户
- 为什么这样组织

它是集成最佳实践的权威文档。具体命令和 API 参考见 [powd-local-api.zh.md](powd-local-api.zh.md)。

## 最终组织

支持的形态只有三类职责：

- `powd`
  - 唯一 daemon
  - 持有活跃 miner runtime、本地 API、event history 和内部 auto loop
- `powctl`
  - 唯一公开前端
  - 持有持久化用户 profile、CLI、TUI 和 MCP bridge
- OpenClaw
  - 注册 MCP bridge
  - 调用 MCP tools
  - 提供更高层 UX

这也明确否定几种方案：

- 为了基础集成去改 OpenClaw 源码
- 把主调度 loop 放进 skill prompt
- 把主调度 loop 放进 OpenClaw plugin 代码
- 在 `powd` 之外再加第二个 adapter daemon

## 为什么 loop 放在 daemon

主 loop 应该放在 `powd`，因为 daemon 已经持有长期运行时关注点：

- 当前活跃 miner runtime
- reconnect 和 runtime 状态迁移
- event buffer
- 趋势指标
- 当前真实生效的运行预算

这条 loop 是确定性代码，不是 LLM prompt loop。

`powctl` 负责用户意图和引导启动，但真正长期执行 miner 的还是 daemon。本地策略因此在 OpenClaw 关闭时仍然成立。

## 适配路径

`powd` 面向宿主的正式入口固定为：

- `powctl mcp serve`
- `powctl mcp config`

`powctl mcp serve` 启动 stdio MCP server。

`powctl mcp config` 输出标准本地 MCP 配置片段，固定包含：

- 绝对路径的 `powctl`
- `args = ["mcp", "serve"]`
- `env = {}`

OpenClaw 只需要注册这个命令，不需要理解 daemon 私有 socket 协议。

如果使用 OpenClaw 自己管理保存的 MCP 配置，支持的注册流是：

1. `powctl mcp config --server-only`
2. `openclaw mcp set powd '<json>'`
3. `openclaw mcp show powd --json`

OpenClaw 的 `mcp set/show/list/unset` 只负责保存配置，不会证明目标 MCP server 当前一定可达。

MCP bridge 只暴露公开业务工具：

- `wallet_set`
- `wallet_show`
- `wallet_reward`
- `miner_status`
- `miner_start`
- `miner_stop`
- `miner_pause`
- `miner_resume`
- `miner_set_mode`

它刻意把账户收益和 miner 运行状态分开：

- `wallet_reward` 是对 pool-service 的外部账户查询
- `miner_status` 仍然只表示本地 daemon 状态

它故意隐藏：

- `daemon.configure`
- 原始 `budget.set`
- 原始 event stream
- pool / pass / worker / strategy 细节
- 只对安装或诊断有意义的命令

## 面向用户的安装路径

面向 OpenClaw 的发布物包含：

- `powd`
- `powctl`

`powd-miner` 仍然保留给底层调试，不属于正常 OpenClaw 安装路径。

正常安装路径是：

1. 安装包
2. 配置一次钱包：
   - `powctl wallet set --wallet-address <addr> [--network main|halley]`
3. 如果要接 OpenClaw，则输出 MCP 片段：
   - `powctl mcp config`
4. 在 OpenClaw 里注册这个 MCP command
5. 之后通过以下入口使用：
   - OpenClaw tools
   - 或 `powctl miner watch`

默认值：

- `network = main`
- 首次 `wallet set` 自动生成 `worker_name`
- 首次 `wallet set` 默认 `requested_mode = auto`

用户不需要管理：

- `login`
- `pool`
- `pass`
- `consensus_strategy`

## 仓库内的 clean 验证

仓库里另外提供一条固定版本的 OpenClaw 验证路径：

1. `nix develop .#openclaw`
2. `scripts/openclaw-smoke.sh`

这条路径会：

- 拉取固定 tag 的 OpenClaw GitHub source tarball
- 用固定的 `node` 和 `pnpm` 在本地构建 OpenClaw
- 把 `OPENCLAW_HOME` 隔离在 `.tmp/openclaw`

它只用于仓库内开发和验证，不是 `powd` 对外的用户安装路径。

## 钱包更新

修改收款地址是正常支持路径的一部分。

用户再次执行 `wallet set` 时：

- 更新 `wallet_address`
- `worker_name` 保持稳定
- 只有显式传 `--network` 时才改变 `network`
- 如果 daemon 已运行，`ctl` 通过私有 API 立即重配它
- daemon 在这次重配前后保持原运行意图

## 为什么这是最佳边界

这个组织方式把边界收得很清楚：

- `ctl` 持有用户意图和持久化 profile
- daemon 持有运行时执行和自动预算控制
- OpenClaw 通过 MCP 使用宿主已经支持的边界

这样第三方集成才现实：

- 不依赖 OpenClaw 源码
- 不需要第二个长期运行的 adapter 进程
- 不会让 `ctl` 持久化一套业务配置、daemon 启动参数又重复一套

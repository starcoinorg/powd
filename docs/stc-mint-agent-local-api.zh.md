# `stc-mint-agent` Local API

## 范围

这份文档描述当前 `stc-mint-agent` 周边已经支持的本地接口：

- `stc-mint-agentctl` 暴露的公开 CLI
- `stc-mint-agentctl integrate mcp` 暴露的 MCP 工具面
- daemon 私有的 Unix socket JSON-RPC
- 本地调用方可依赖的稳定状态字段

OpenClaw 集成的整体理由见 `docs/stc-mint-agent-openclaw-integration.zh.md`。

## 用户模型

公开入口始终是 `stc-mint-agentctl`。

持久化用户配置只由 `stc-mint-agentctl` 持有，不由 daemon 持有。配置内容为：

- `wallet_address`
- `worker_id`
- `requested_mode`
- `network`

支持的 `network` 值：

- `main`
- `halley`

以下值不持久化，而是在 daemon 内部派生：

- `login = wallet_address.worker_id`
- `pool`
- `pass`
- `consensus_strategy`

## 公开 CLI

`stc-mint-agentctl` 是唯一公开的人类/脚本入口。

### Wallet 命令

- `stc-mint-agentctl wallet set --wallet-address <addr> [--network main|halley]`
- `stc-mint-agentctl wallet show`
- `stc-mint-agentctl wallet reward`

语义：

- `wallet set` 是唯一的钱包写命令
- 首次执行时会创建稳定的 `worker_id`
- 后续执行只更新 `wallet_address`
- `worker_id` 保持不变
- 首次执行时 `--network` 默认是 `main`；后续未传时保持原 network
- 如果 daemon 正在运行，`wallet set` 会立即对 daemon 重配
- `wallet reward` 是独立的外部账户查询，数据来自 pool-service
- `wallet reward` 使用持久化的 `wallet_address + network`，不依赖 daemon

### Miner 命令

- `stc-mint-agentctl miner status`
- `stc-mint-agentctl miner start`
- `stc-mint-agentctl miner stop`
- `stc-mint-agentctl miner pause`
- `stc-mint-agentctl miner resume`
- `stc-mint-agentctl miner set-mode <auto|conservative|idle|balanced|aggressive>`
- `stc-mint-agentctl miner watch`

mode 语义：

- `auto`
  - 保持 `requested_mode = auto`
  - 让 daemon 内部计算更细粒度的 `effective_budget`
  - 不公开 governor 内部调节旋钮
- `conservative|idle|balanced|aggressive`
  - 面向用户的固定 preset
  - 每个 preset 都映射到固定的 `effective_budget`

`pause` 和 `stop` 不会丢掉 `auto`。它们只会让 auto 进入 held 状态；`resume` 和 `start` 会清除这个 hold。

### Integrate 命令

- `stc-mint-agentctl integrate doctor`
- `stc-mint-agentctl integrate mcp-config`
- `stc-mint-agentctl integrate mcp`

语义：

- `doctor` 检查持久化钱包配置、daemon 可达性和当前运行状态
- `mcp-config` 输出 OpenClaw MCP 注册片段
- `mcp` 启动 OpenClaw 拉起的 stdio MCP server

## MCP 工具面

`stc-mint-agentctl integrate mcp` 暴露以下业务工具：

- `wallet_set`
- `wallet_show`
- `wallet_reward`
- `miner_status`
- `miner_start`
- `miner_stop`
- `miner_pause`
- `miner_resume`
- `miner_set_mode`

它不暴露：

reward 被刻意和 `miner_status` 分开：

- `miner_status` 保持纯本地 daemon 状态
- `wallet_reward` 通过外部 HTTP 查询 pool-service

它不暴露：

- 原始 `budget.set`
- 原始 `events.stream`
- `doctor`
- `mcp-config`
- daemon 私有的初始化/重配置细节

CLI 和 MCP 共用同一套底层业务命令。它们只是不同 transport，不是两套状态机。

## Daemon 私有 JSON-RPC

`stc-mint-agent` 通过 Unix socket 暴露 daemon 私有 JSON-RPC，供 `stc-mint-agentctl`、dashboard 和诊断使用。

当前方法：

- `daemon.configure`
- `daemon.shutdown`
- `miner.start`
- `miner.stop`
- `miner.pause`
- `miner.resume`
- `miner.set_mode`
- `status.get`
- `status.capabilities`
- `status.methods`
- `events.since`
- `events.stream`

`daemon.configure` 是私有方法，输入为：

- `wallet_address`
- `worker_id`
- `requested_mode`
- `network`

daemon 在内存中根据这份 profile 派生 `login`、`pool`、`pass` 和 `consensus_strategy`。

## 启动模型

`stc-mint-agent` 以空白 daemon 形态启动，不接受 `--login`、`--pool` 之类公开业务参数。

任何需要 daemon 的 `stc-mint-agentctl` 命令都统一走：

1. `ctl` 读取持久化 profile
2. 如果 daemon 不在，则启动空白 `stc-mint-agent`
3. `ctl` 调用 `daemon.configure(profile)`
4. `ctl` 再执行请求的业务动作

如果本地还没有 profile，必须先执行 `wallet set`。

## 状态模型

主要读取模型是 `status.get`。

关键字段：

- `state`
- `connected`
- `pool`
- `worker_name`
- `requested_mode`
- `effective_budget`
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
- `system_cpu_percent`
- `system_memory_percent`
- `system_cpu_percent_1m`
- `system_memory_percent_1m`
- `auto_state`
- `auto_hold_reason`
- `last_error`

语义：

- `requested_mode` 表示用户选择的 mode
- `effective_budget` 表示当前真实生效的运行预算
- 当 `requested_mode = auto` 时，`effective_budget` 会随时间变化
- `auto_state` 只有 `inactive`、`active`、`held` 三种
- 只有在 `auto_state = held` 时，`auto_hold_reason` 才会出现

## TUI

`stc-mint-agentctl miner watch` 是面向人的 dashboard。

它显示：

- miner 状态和连接情况
- `requested_mode`
- `effective_budget`
- `auto_state`
- 当前值和 1 分钟平均的系统 CPU / memory 使用率
- hashrate 和趋势指标
- 最近事件
- 最后一次错误

它支持：

- start
- stop
- pause
- resume
- mode 切换
- 钱包更新

TUI 只是同一套业务命令的本地展示和输入层。

# `stc-mint-agent` 本地接口

## 1. 目标

当前实现的目标很窄：

- `stc-mint-agent` 是唯一长驻 daemon
- `stc-mint-agentctl` 是唯一前端
- OpenClaw 通过 `stc-mint-agentctl mcp` 接入
- 人类通过 `stc-mint-agentctl` 和 `dashboard` 接入
- 用户只关心 `wallet_address`
- 默认挖 `main`

这份文档只描述当前已经落地的本地接口，不讨论补贴、增长、agent 专用 `stratumd`，也不讨论远程操作。

## 2. 用户模型

用户只需要配置一件事：

- `wallet_address`

系统内部自动处理：

- 生成并持久化稳定 `worker_id`
- 组合最终 login：`wallet_address.worker_id`
- 选择默认 mainnet pool
- 选择默认 `consensus_strategy`
- 在需要时自动拉起 `stc-mint-agent`

用户可以随时修改收款地址：

- 只更新 `wallet_address`
- `worker_id` 保持不变
- 如果 daemon 正在运行，新的 login 立即通过受控重启生效

本地持久化状态只保存：

- `wallet_address`
- `worker_id`

不引入复杂矿工配置文件。

## 3. 组件边界

### 3.1 `stc-mint-agent`

`stc-mint-agent` 负责：

- 持有 miner core
- 维护 JSON-RPC 本地接口
- 维护生命周期状态机
- 暴露状态、趋势指标和事件缓冲

它不负责：

- OpenClaw 调度策略
- 用户交互
- TUI
- MCP 协议

### 3.2 `stc-mint-agentctl`

`stc-mint-agentctl` 是统一前端，负责三类入口：

- 普通 CLI
- `mcp` 子命令
- `dashboard` 子命令

它通过本地 Unix socket 连接 `stc-mint-agent`，并在 daemon 不存在时自动拉起。

### 3.3 OpenClaw

OpenClaw 不直接连 miner，也不直接调 CLI 字符串。

它注册：

- `stc-mint-agentctl mcp`

然后通过 MCP tools 调用本地接口。

调度循环也放在 OpenClaw：

- 读取 `status` 和 `events_since`
- 结合系统 CPU、内存、用户活跃、电源状态
- 决定何时 `set_mode`、`pause`、`resume`、`start`、`stop`

## 4. MCP 工具面

`stc-mint-agentctl mcp` 只暴露安全工具：

- `setup`
- `set_wallet`
- `status`
- `start`
- `stop`
- `pause`
- `resume`
- `set_mode`
- `events_since`

不暴露：

- 原始 `budget.set`
- 原始 `events.stream`
- 矿池、密码、worker、network 选择

### 4.1 `setup`

输入：

- `wallet_address`

效果：

- 保存钱包地址
- 若不存在则生成稳定 `worker_id`
- 返回当前配置摘要

### 4.2 `set_wallet`

输入：

- `wallet_address`

效果：

- 更新钱包地址
- 保持 `worker_id` 不变
- 若 daemon 正在运行，则受控重启并让新 login 立即生效

### 4.3 `set_mode`

只允许 preset mode：

- `conservative`
- `idle`
- `balanced`
- `aggressive`

不允许上游直接传原始 budget。

## 5. CLI 与 TUI

### 5.1 CLI

当前人类和脚本入口：

- `setup --wallet-address ...`
- `set-wallet --wallet-address ...`
- `status`
- `start`
- `stop`
- `pause`
- `resume`
- `set-mode <mode>`
- `doctor`
- `mcp-config`

`doctor` 用来检查：

- 钱包是否已配置
- `worker_id` 是否存在
- daemon 是否可达
- 当前 miner 状态和最近错误

`mcp-config` 用来输出可直接注册到 OpenClaw 的 MCP 配置片段。

### 5.2 Dashboard

`stc-mint-agentctl dashboard` 是给人类的本地 TUI。

v1 展示：

- 当前状态
- 连接状态
- hashrate / `hashrate_5m`
- accepted / rejected / submitted
- `reject_rate_5m`
- 当前 budget
- 最近事件
- 最近错误

v1 交互：

- `s` start
- `x` stop
- `p` pause
- `r` resume
- `1` conservative
- `2` idle
- `3` balanced
- `4` aggressive
- `w` 修改钱包地址
- `q` 退出

## 6. daemon 自动拉起

MCP、CLI 和 dashboard 在需要 daemon 的操作前，都会先探测本地 socket。

若 daemon 不存在：

1. 检查 `wallet_address` 和 `worker_id` 是否已就绪
2. 未就绪则要求先 `setup`
3. 已就绪则自动启动 `stc-mint-agent`

自动启动参数全部内部推导：

- 默认 network = `main`
- 默认 pool = mainnet pool
- 默认 `consensus_strategy` = mainnet 默认算法
- login = `wallet_address.worker_id`

这些细节对用户和 OpenClaw 都隐藏。

## 7. 读路径

### 7.1 状态

主要读接口：

- `status.get`
- `status.capabilities`
- `status.methods`

其中 `status.get` 至少包括：

- `state`
- `connected`
- `pool`
- `worker_name`
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
- `current_budget`
- `last_error`

### 7.2 事件

面向 OpenClaw 的主事件接口是：

- `events.since`

它返回：

- `next_seq`
- `events`

OpenClaw 用 request-response 轮询，不用 `events.stream`。

`events.stream` 只保留给人类调试和 CLI 长连接监听。

## 8. 当前约束

当前 deliberately 保持这些约束：

- 默认只面向 `main`
- 用户流程里不暴露 `halley`
- OpenClaw 不直接改原始 budget
- 自动调度不进 miner，也不进 MCP server
- 只有 `stc-mint-agent` 是长驻进程，不再引入第二个 adapter daemon

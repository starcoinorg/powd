# `stc-mint-agent` 与 OpenClaw 集成

## 1. 文档目的

这份文档只回答一组组织问题：

- 动态调度 loop 应该放哪里
- OpenClaw 应该通过什么方式接入
- 用户安装路径应该怎样收口
- 为什么要这样组织

这是一份**目标组织文档**，不是当前本地接口逐项参考。接口细节仍看：

- `docs/stc-mint-agent-local-api.zh.md`

## 2. 最终结论

最佳实践收成四条：

- `stc-mint-agent` 是唯一长期持有业务状态的 daemon
- 动态调度 loop 放在 `stc-mint-agent` 内部的 `governor`
- OpenClaw 通过 `stc-mint-agentctl mcp` 这个 **stdio MCP** 入口接入
- `skill` / `plugin` 只做发现、安装和使用引导，不承载业务主循环

不采用这些方案：

- 不要求修改 OpenClaw 源码
- 不把主 loop 放进 OpenClaw 内部代码
- 不把主 loop 写进 skill prompt
- 不再新增第二个 adapter daemon

## 3. 组件组织

### 3.1 `stc-mint-agent`

`stc-mint-agent` 持有：

- miner core
- `wallet_address`
- 稳定 `worker_id`
- 派生 login：`wallet_address.worker_id`
- runtime 状态、趋势指标、事件缓冲
- `governor` 调度状态

`governor` 是 daemon 内部的确定性调度子系统。它负责：

- 周期采样系统负载和 miner 健康状态
- 决定 `conservative / idle / balanced / aggressive`
- 维护升降档冷却和冻结状态
- 在用户手动 override 后暂停自动调度

### 3.2 `stc-mint-agentctl`

`stc-mint-agentctl` 是统一前端，只承担三类事情：

- 人类 CLI
- `dashboard` TUI
- `mcp` bridge

它不持有第二份业务状态，不运行第二个调度 loop。

### 3.3 OpenClaw

OpenClaw 作为宿主，只负责：

- 注册 `stc-mint-agentctl mcp`
- 调用 MCP tools
- 展示状态
- 做手动 override
- 通过 skill / plugin 改善发现和 UX

OpenClaw 不需要改源码，也不需要承担 miner 的长期业务状态。

## 4. loop 为什么放在 daemon 里

主 loop 放在 `stc-mint-agent` 而不是 OpenClaw，有四个原因：

- 第三方接入不能假设能长期修改和维护 OpenClaw 源码
- `wallet_address`、`worker_id`、login、矿池连接和事件缓冲都已经在 daemon 内，loop 贴着这些状态最自然
- `skill` 是提示层，`plugin` 是接入层，它们都不适合承载需要持久状态和冷却逻辑的业务循环
- 用户关闭 OpenClaw 之后，miner 仍应保持稳定的本地策略，而不是失去调度能力

OpenClaw 仍然有价值，但角色收窄为：

- 本地 MCP 客户端
- 用户入口
- 手动 override 和可视化入口

## 5. 安装与分发

面向用户的 v1 分发物固定为三个 binary：

- `stc-mint-miner`
- `stc-mint-agent`
- `stc-mint-agentctl`

用户路径固定为 wallet-first：

1. 安装发行包
2. 执行一次：
   - `stc-mint-agentctl setup --wallet-address <addr>`
3. 如需 OpenClaw，执行：
   - `stc-mint-agentctl mcp-config`
4. 把生成的 MCP 配置注册到 OpenClaw
5. 日常使用：
   - OpenClaw 调 MCP
   - 或 `stc-mint-agentctl dashboard`

用户默认只关心：

- `wallet_address`

系统自动处理：

- 默认 network = `main`
- 默认 pool = mainnet pool
- 默认算法 = mainnet 默认算法
- 自动生成稳定 `worker_id`
- 自动派生 login
- daemon 不在时自动拉起

用户修改收款地址时：

- 只改 `wallet_address`
- `worker_id` 保持不变
- 新 login 立即通过热切换或受控重启生效

## 6. OpenClaw 适配方式

OpenClaw 的正式适配面固定为：

- `stc-mint-agentctl mcp`

这是一个 stdio MCP server。OpenClaw 只需要注册它，不需要理解 miner 内部协议。

MCP 对外只暴露安全工具：

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
- 后续 governor 只暴露治理相关工具，不直接暴露原始 `budget.set`

`skill` 和 `plugin` 的角色固定为可选增强：

- `skill`：教模型何时调用哪些 MCP tools
- `plugin`：帮用户自动注册 MCP、暴露更好的 UI 或安装引导

它们都不应该持有 miner 主状态，也不应该跑业务主 loop。

## 7. 面向用户的最终体验

用户最终看到的产品心智应当只有两条：

- 给我一个 `wallet_address`
- 默认跑 main，其他我不用管

人工用户主要用：

- `stc-mint-agentctl dashboard`
- 少量 CLI 命令

OpenClaw 用户主要用：

- 已注册的 MCP tools

两条路径共享同一个 daemon、同一份状态和同一套调度规则。

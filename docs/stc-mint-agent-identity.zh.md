# `stc-mint-agent` 身份与最小协议

## 1. 文档目的

这份文档只讲 agent 侧本地身份和最小私有协议，不讲补贴公式，不讲 payout，也不讲完整账户系统。

它回答四个问题：

- 收款地址是什么
- `worker_name` 承担什么身份
- `agent_auth` 最小握手怎么做
- 这层协议和正常 Stratum 流程是什么关系

## 2. 核心结论

当前方案只保留两种外部身份：

- `wallet_address`
  - 只负责收款
- `worker_name`
  - 直接作为 `stc-mint-agent` 的本地身份

因此，agent 侧登录继续沿用现有格式：

- `wallet_address.worker_name`

在这条线里：

- `wallet_address` 管钱
- `worker_name` 管本地进程身份

这里不再额外引入单独外露的 `agent_id`。

## 3. 身份模型

### 3.1 `wallet_address`

`wallet_address` 是收款地址。

要求很简单：

- 可以本地生成
- 不需要先和 Starcoin 网络交互
- 当前阶段只承担收款路径，不承担最小认证职责

### 3.2 `worker_name`

`worker_name` 直接作为本地身份。

它由 `stc-mint-agent` 首次启动时生成，并在本地持久化。后续重启继续复用，不应每次启动都变化。

这也是为什么 agent 侧不能把 `worker_name` 当纯显示标签：它需要承担稳定 identity。

同时它仍要满足现有矿池 worker 规则：

- 小写
- 只包含字母、数字、`_`、`-`
- 长度不超过当前服务端限制

## 4. 最小协议

ASIC 那侧走标准 Stratum 协议，不适合再叠私有握手。agent 侧不同，客户端和服务端都在我们控制下，因此允许增加一步很薄的 `agent_auth`。

最小流程如下：

1. `stc-mint-agent` 连接 agent 专用 `stratumd`
2. 服务端返回 challenge
3. 客户端返回：
   - `worker_name`
   - `agent_pubkey`
   - `sig(challenge || worker_name)`
   - 可选版本号
4. 服务端验签通过后，才允许继续进入正常 Stratum 流程
5. 客户端再发送正常 `login`，格式仍然是 `wallet_address.worker_name`
6. 服务端要求 `login` 里的 `worker_name` 与刚才 `agent_auth` 里的一致

这层握手的作用只有一个：

- 证明这条连接确实持有某个稳定的本地身份私钥

它不证明：

- 这一定是 AI agent
- 这绝对不是 ASIC

## 5. 边界

### 5.1 `stc-mint-agent` 负责

- 生成并持久化 `worker_name`
- 生成并持久化 `agent_keypair`
- 管理 `wallet_address`
- 对 challenge 做签名
- 在认证通过后继续正常挖矿

### 5.2 agent 专用 `stratumd` 负责

- 下发 challenge
- 校验签名
- 只放行通过 `agent_auth` 的连接
- 在放行后继续正常 `login / job / submit`

### 5.3 这里不负责

这份设计不解决：

- 钱包归属证明
- session token
- 绑定表或账户系统
- 补贴公式
- 异常算力阈值
- 完整反 ASIC 方案

## 6. 结论

这条线的最小 identity 方案很简单：

- `wallet_address` 负责收款
- `worker_name` 负责本地进程身份
- `agent_auth` 只负责给 agent 专用 `stratumd` 增加一层最小边界

这样既不需要把 ASIC 协议改复杂，也不需要一开始就做重认证系统。

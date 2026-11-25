# 新架构进度

## 当前状态

截至 2026-11-26（台北时间），原生渠道和控制台的整合候选 E 已合入主线，渠道标签草稿和编辑器焦点修复也已合入。生产切流仍缺最终停写快照、资金对账和恢复回执。以下结论基于源码与已有验收记录，目标主机的实际版本以带时间的生产回执为准。[主线 04eb2fce](https://github.com/TokenNotIncluded/lmm-api-source/commit/04eb2fce5fc549e53b5ddd5933ba10db63feb0d7) · [整合 PR #18](https://github.com/TokenNotIncluded/lmm-api-source/pull/18)

### 账号与 Key

固定候选上已有账号密码、OAuth 回调与拒绝、会话刷新、Passkey 和旧 Key 的分项证据。五个 public client 各有独立开关和授权额度。

3 个旧 OAuth 刷新家族仍为 `BLOCKED_OAUTH_CONTINUATION_REQUIRED`。用最终同源快照验证已有账号登录、跨重启恢复和 Key 权限。缺刷新 Cookie、旧 MFA 恢复码、特殊 Key 重签和其他 Passkey 格式仍有缺口。Google 登录按决定等正式域名确定后再接，届时需重新核对回调 URI。[身份与权限](core-identity.md) · [会话恢复记录](reports/lmm-auth-session-recovery-2026-11-19.md)

### 计费与历史权益

Core 固定付款方、报价与请求合同，负责预留和最终记账。隔离 Core、Go 和 PostgreSQL 已验证重复通知只入账一次。固定制品验证了流式中断后保留预留、沿原合同恢复结算。

停写后逐户核对钱包、欠额、推广余额、托管、订阅、订单和 Key。历史对账仍有 81 笔充值缺字段、13 笔 EPay 缺冻结付款快照、3 笔孤儿记录。另有 3 笔合计 USD 4.44 的旧订单待确认归属和支付方终态。60 笔历史订阅订单保留 `UNKNOWN`，15 份非活跃订阅待复核。各组记录须按原始标识匹配。[历史支付义务](legacy-payment-obligations.md) · [未决负债核对](legacy-pending-liability-reconciliation.md) · [计费合同](model-billing.md)

### 钱包、订阅与商店

旧订单按成交时冻结的渠道与条款恢复。Go 保存订单、通知和重试状态，经 Core RPC 请求入账。固定组合已覆盖人民币模拟充值、付费提货邮件及部分商店购买、退款和重放。钱包显示仍按付款与账本原单位核对。[钱包历史](wallet-history.md)

分开验收“接收旧单回调”和“开放新 checkout”。钱包渠道 5001/5003 共用商店，切换前须按订单归属核对旧 Go webhook 和新回调。5004 EPay 的旧折扣和固定比例差异待处理，5002 测试订阅有独立开关。补齐外部商店退款的原交易、通知/查询、收入回冲、重启恢复，以及各 provider 的订阅周期、取消和续接。钱包 [PR #19](https://github.com/TokenNotIncluded/lmm-api-source/pull/19) 的展示修复按其最终提交另验。[支付网关](payment-gateways.md) · [资金合同](payment-money-contract.md)

### 渠道与模型

固定 Core、真实 HTTP/PG 与合成上游验证了 OpenAI Chat、Responses、Anthropic、Gemini 四种非流式调用的 Key 生命周期。候选全部不兼容时零派发、零预扣。渠道支持多 Key 选择快照及 Sub2API 余额观测。[Key 与协议](reports/lmm-channel-key-protocol-2026-11-19.md) · [请求选路与结算](reports/lmm-request-aware-http-2026-11-19.md)

按客户端协议、上游协议和输出模式逐格验收。Responses HTTP 连续性与 compact 已有固定输入的独验。WebSocket 生产接线、hosted tools/fileData、独立 TTS/STT/translation、双向 Realtime、异步视频/音乐仍有缺口。`/api/task`、`/api/mj` 仍提供历史读取。旧合同缺少快照时，管理员日志将所选 Key 记为 `unknown`。供应商异步执行和日志闭环见 [#853](https://github.com/TokenNotIncluded/lmm-api/issues/853)。[协议转换验收](quality.md#协议转换验收)

### 控制台与扩展

主线已有渠道筛选、批量操作、编辑器和权限导航。现有浏览器收据标明角色、视口和使用的后端。Extore 的 GNU 联调覆盖了 HTTPS、SQLite 和共享库。

管理员统计/财务页、时区和 CSV 仍须与旧口径逐项对照。Extore 须验证已安装 Alpine 包、真实 CoreSession、Store PostgreSQL、跨店授权和浏览器。管理员审核、助手/MCP/工具/HumanGrant 也各需最终组合回执。[站点财务来源](site-finance-source.md) · [Extore 验收合同](extore.md#验收合同)

## 首发范围

首发要让既有用户继续使用账号和 API Key，能够调用文字、图片模型，按实际用量结算，使用含人民币在内的钱包充值，完成商店购买、库存交付、提货、退款，并承接订阅和既有权益。允许重建旧商店商品、库存及工具市场内容。已有订单的付款、退款和持续权益须保留原始依据。8 个在用模型已书面延期，Gemini 音频延期，zcall 更新留到下版。[在用模型延期清单](launch-scope-in-use-deferral.md) · [产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md) · [支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)

## 验收状态

正式切流需在同一冻结候选上取得以下回执。

1. **备份与恢复。** 在同一冻结点取得 Core 与 8 个 Extensions 数据库的完整备份，实际恢复并核对持久化状态、权限和服务读回。旧在线副本、单库回归和旧快照 `FINAL_20261111T120751Z` 各有自己的验证范围。[离线升级](upgrade-offline.md) · [生产预检](core-cutover-preflight.md)
2. **停写与开账。** 固定最终来源，生成同源身份、Key、钱包与旧权益预览，由 owner 审核并显式提交。逐笔保留差额和未决义务，记录目标账本与来源的对应关系，再验证运行账户权限和单一资金写入者。[财务开账](legacy-financial-opening.md) · [生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)
3. **数据兼容与回退。** 新 reader 可读旧 D 的请求合同。D 的严格 reader 会拒绝带 `credential_selection` 的新合同。第一份新合同写入后，回退下限是能读该字段的版本。若须保留 D 的窗口，先交付 reader-only 版本、升级所有读取方，再批准新 writer。CBC 回退另有未决约束。保留原始合同字节与摘要。[旧候选的兼容性说明](https://github.com/TokenNotIncluded/lmm-api-source/blob/04eb2fce5fc549e53b5ddd5933ba10db63feb0d7/docs/progress.md#当前状态)
4. **发布与目标主机验收。** 对同一组 Core、Extensions、CLI 和静态 Web 固定源码、依赖、制品、签名及安装清单，核验 CI、目标主机安装、启动、路由与恢复。候选 `a9fab477` 已有自身的 CI 和三档资源记录。新组合还须验证长稳、容量、故障恢复、各角色与窄屏浏览器、CLI 授权续接和多平台分发。[打包与分发](release-architecture.md) · [独立业务与资源验收](quality.md#独立业务与资源验收)

### Core 88 候选

[PR #16 的冻结提交 eb1cdcf6](https://github.com/TokenNotIncluded/lmm-api-source/tree/eb1cdcf6acce9a5d32ae83d3bde7ff69b9e6371d) 纳入每周优惠的持久决定、领取和 owner 历史导入，导入关联原券。管理确认绑定会话、权限及原草稿，结果未知时查询原操作。候选修复了首位 owner 种子检查、支付测试 schema/运行角色、私有技能取消和重置链接脚本的回归。已记录的隔离 PostgreSQL、Web 和合成浏览器检查各自绑定候选输入。专用支付长流程仍被忽略，最终制品与完整 CI、真实支付恢复、全域迁移和资源验收仍待核对。正式安装版本的组件数由目标二进制的 `lmm-core schema-manifest` 读回。安装与升级说明见[开发指南](development.md)。[候选原始记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/eb1cdcf6acce9a5d32ae83d3bde7ff69b9e6371d/docs/progress.md#core-88-整合候选2026-11-23开发验收中)

冻结提交 `eb1cdcf6` 的身份协议检查有一项失败。升级测试的父进程在子进程写完监听地址前读到了空文件。修复改为写完后再原子重命名。两个 Rust 进程的复现验证了修复后的回环请求，真实 PostgreSQL 升级用例仍需在包含修复的提交上通过。[此前的回归修复记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/eb1cdcf6acce9a5d32ae83d3bde7ff69b9e6371d/docs/progress.md#L5-L11) · [失败的数据库检查](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/45656486910/job/139354444972) · [就绪文件修复](https://github.com/TokenNotIncluded/lmm-api-source/pull/28)

## 证据入口

| 需要追查的事实 | 从哪里查 |
| --- | --- |
| 单次请求从派发到扣费、流式中断怎样恢复 | [请求选路实测](reports/lmm-request-aware-http-2026-11-19.md) · [固定制品的恢复收据](reports/lmm-launch-validation-2026-11-21.json) · [Responses 连续性](responses-continuity.md) |
| 旧账号、订单与账本由谁接手，最后一次核验了什么 | [会话恢复](reports/lmm-auth-session-recovery-2026-11-19.md) · [旧付款证据](legacy-provider-verification.md) · [原始私有回执目录索引](https://github.com/TokenNotIncluded/lmm-api-source/blob/04eb2fce5fc549e53b5ddd5933ba10db63feb0d7/docs/progress.md#证据入口) |
| 历次成功、失败、忽略和未运行分别属于哪个版本 | [精简前的完整原文](https://github.com/TokenNotIncluded/lmm-api-source/blob/04eb2fce5fc549e53b5ddd5933ba10db63feb0d7/docs/progress.md) · [逐项验收矩阵（REL、AUTH、MODEL、SUB、PAY、STORE、APP、CLI、CHECK 等）](https://github.com/TokenNotIncluded/lmm-api-source/blob/04eb2fce5fc549e53b5ddd5933ba10db63feb0d7/docs/progress.md#验收状态) · [历史资料索引](archive.md) |

## 范围与维护

本页只更新会影响用户、验收或切流决策的变化。记录结论时附上具体提交、运行环境和证据入口，区分开发自测、独立验收、主线合并及生产回读。逐次执行日志、私有数据与凭据留在相应受限证据位置。Extore 的主分支会自动部署，双侧集成按[既有约束](extore.md)进行。

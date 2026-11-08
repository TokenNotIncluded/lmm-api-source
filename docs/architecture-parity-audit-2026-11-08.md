# LMM API 新旧架构完成度审计

检查日期：2026-11-08。执行记录按 UTC 保存。产品源码固定为 `53c945dccb3ef1a105261bc6a91c01dfc6f26191`；文档提交不改变被审产品基线。

## 1. 结论

**新架构的主要结构和不少核心业务已经落地，完整功能对齐、真实数据迁移及最终联合验收尚未完成。** 身份、权限、账本、模型执行、多个充值入口和部分商业闭环已有真实实现；仍有旧功能缺失、页面与后台断口，以及可以复现的质量和交付阻断。

下文按明确的行为列出待关闭工作包。工作包大小不等、部分共用基础和证据，数量不能换算成完成百分比、互不重叠的工时或潜在缺陷总数。本次不沿用历史“约60%/40%”：项目验收合同本身要求逐行为证明，禁止用目录数、测试数量或健康接口代替完整完成度。[生产验收合同][acceptance]

本次识别并整理出 **39 组推进工作**：

| 类别 | 数量 | 编号 | 含义 |
| --- | ---: | --- | --- |
| 旧功能对齐 | 26 | F01–F26 | 实现缺失、行为受限或前后端未闭合 |
| 新产品目标 | 1 | N01 | CLI 完整承诺，单列以免当作旧服务端回退 |
| 迁移与验收 | 9 | V01–V09 | 当前最终组合的证据闭环 |
| 工程交付整改 | 3 | D01–D03 | 边界、工作流、制品与状态 |
| **合计** | **39** | | **工作分组数，不是完成率分母** |

最先需要处理的可复现问题是：边界测试 **32 项中 6 项失败**；完整质量门禁在计划阶段被已过期且仍 open/critical 的 **QD-001** 阻断；部分交付工作流仍绑定旧仓库和旧 WIP 分支；分发流程检查 JSON 示例，实际 Dockerfile 安装 TOML。它们分别是当前合同审查和交付链问题，不应直接解释成已发生越权或资金损失。[边界检查][boundary]、[债务登记][debt]、[镜像流程][images-workflow]、[分发流程][distribution-workflow]

## 2. 基线与方法

| 对象 | 固定输入 | 用途 |
| --- | --- | --- |
| 新架构 | [lmm-api-source 53c945dc][source] | Rust Core、Go Extensions、Web、测试、配置和交付工具 |
| 配方与完整需求 | [lmm-api 8f11ef82][recipes] | A01–A10、S11、支付矩阵；源码 gitlink 与 53c945dc 一致 |
| 旧稳定发布 | [go-v0.2.118][legacy-release]，源码 `cc52aa195c6f2a01d703a1a407be73c31b89867d` | 完整旧 Go 路由、控制器、模型与服务，位于 `apps/api-go/` |
| 支付专项旧基线 | `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` | 配方固定的 [PAYMENT_PARITY.md][payment-parity]；不假定与稳定发布提交完全相同 |
| CLI | `TokenNotIncluded/lmm@f86ec5e605b00946a7227db78519c482476365e3` | 已初始化并核对当前锁定子模块的实现和产品边界 |

本次旧 `traditional` 分支查询返回不存在，故使用可重现的发布提交，并结合完整配方要求。Nov7 Drive 审计冻结于 source `787c8cc5` / recipes `c7b2f967`，Oct30 功能报告冻结于 source `7520d0bc`，都早于本轮；只将它们作为待核清单，不沿用其缺口数量或通过结论。[Nov7 保存报告][prior-report]、[Oct30 功能报告][prior-parity]

检查顺序为：旧行为是否真实存在 → 新入口是否挂载 → 是否调用真实存储、权限或资金逻辑 → 当前 Web 是否使用该入口 → 测试覆盖什么 → 是否有绑定当前候选的制品和运行证据。仅有接口名、页面、测试源码、目录或 debug fixture，不构成完成证明。

本次完成全域源码和历史证据审计，并执行直接相关的离线检查；未运行完整 Rust/Go 构建、全站浏览器、外部付费供应商或生产迁移。对应范围在下文保留为待验收。这是一份完成度审计，不是当前产品的完整运行验收证书。

## 3. 已经落地的进展

旧单体集中处理登录、模型、扣费、支付和业务后台。新产品保留三个程序：`lmm-core` 负责核心身份、权限、账本与模型请求权威；`lmm-extensions` 承载业务模块和外部 IO；`lmm` 是客户端，Web 单独作为静态产物。迁移完成的关键是业务跨过这些边界后仍然做对。[交付结构][release-architecture]、[旧路由][old-api]

下列内容已经有实际实现，应从“整个模块未开发”的旧欠账中删除。这里的已实现不等于当前版本已完成独立运行验收。

| 进展 | 当前事实 | 范围限制 |
| --- | --- | --- |
| 文本模型执行、流式、路由与限流 | Chat/Responses/Messages/Gemini 文本入口、保守重试、Auto、用户/分组限制和请求账本已有 | 完整多模态、实时、异步媒体及更多旧同步行为尚未齐 |
| Embeddings / Rerank | 实际入口合并进 ModelGateway；鉴权、限流、预扣、回执与恢复已有 | 不能扩大为所有供应商和所有形状已验收 |
| 原生渠道与价格管理 | CRUD/copy、配置发布、价格草稿/预览/发布、加密密钥库、模型/vendor 元数据已有 | 高级运维、自动 key 轮转和真实推理探测仍需补 |
| 身份与 Passkey | 常规身份、会话、MFA、多种外部登录和 WebAuthn 注册/登录有真实实现 | 通用 OIDC、WeChat 和旧 OIDC provider 仍有差距 |
| 钱包、活动与余额订阅 | 赠送、兑换码、签到、红包、订阅购买/续期/取消/未用退款接实际账本 | 平台充值折扣码是另一个业务；外部订阅仍欠 |
| 推荐奖励转钱包 | `affiliate.rs` 在浏览器确认后执行 `ClaimAffiliate` 写账；主进程挂载、钱包有入口 | 真实旧权益来源、单位与迁移仍待对账 |
| 多 provider 钱包充值 | EPay、Stripe、Creem、Waffo Global、Pancake 有真实 adapter/桥接；Coinbase 是新增能力 | 钱包充值、订阅支付、商店收款分别验收 |
| 退款与恢复 | Stripe、Waffo Global 退款和 Core `ReleaseRefund` 已有；Waffo 支持全额/部分退款 | Pancake、外部商店退款及最终 Go main/真实 provider 仍欠 |
| 商店与 Extore | 余额/免费购买、优惠券、累计限购、登录买家提货、库存密封密钥、点赞、Extore 授权/同步/只读 UI 已接线 | 外部收款、旧商店细节及 Extore 两端真实验收仍需补 |
| 工具与助手基础 | 持久会话、工具调用、search/fetch、冻结价格、供应商回执和资金权限已有 | 完整助手体验、工具管理及跨域真实验收分别核对 |

依据：[ModelGateway][new-model]、[Input API][input-api]、[渠道][channels]、[Passkey][passkeys]、[活动账本][promotions]、[订阅账本][subscriptions]、[推荐转账][affiliate]、[钱包 checkout][payment-checkout]、[Waffo 退款][waffo-refunds]、[退款 RPC][refund-rpc]、[Extore][extore]。

当前可枚举 **56 个 Core schema component、110 个唯一 `createFileRoute` 声明、60 个 Web feature 目录、735 个 Web 测试文件**；Go 宿主允许模块列表含 10 项。路由包含布局/跳转，动态页面还会展开多个操作；测试文件数不是实际执行通过数。这些数字只用于确定检查范围。[schema][schema]、[Web routes][web-routes]、[Go 宿主][extensions-run]

## 4. 二十六组旧功能对齐工作

P1 表示优先恢复核心业务、资金或身份相关能力；P2 表示可随后推进的管理/体验能力。二者均属于原完整范围，不因优先级较低而自动退出验收。

### 模型、渠道与助手

| 编号 | 优先级 / 工作包 | 当前差距 | 关闭条件 | 证据 |
| --- | --- | --- | --- | --- |
| F01 | P1 · 多模态与原生协议范围 | Chat/Responses 图像输入、Anthropic image/cache_control/hosted server tools、Gemini inlineData/fileData 等被当前 parser 明确拒绝。独立图片生成/编辑不能代替聊天视觉。 | 补输入/响应、usage、价格与恢复合同，验证真实可用及非法输入拒绝。 | [请求解析][request-parser]、[Anthropic][anthropic]、[Gemini][gemini] |
| F02 | P1 · 完整 Responses 与旧同步入口 | `store=true` 被拒绝，`previous_response_id` 不在白名单；旧 completions、responses/compact、alpha/search、decisions、moderations 没有等价公共入口。 | 补有状态响应、工具、持久化/重放/计费及需要保留的旧 API；以实际行为对齐。 | [旧模型入口][old-relay]、[新入口][new-model]、[Responses 限制][response-parser] |
| F03 | P1 · 独立音频与双向实时 | 独立 TTS/STT/translation、Responses WebSocket、Realtime/native voice/live 未在新网关挂载。当前 Chat audio 和持久化流式已实现。 | 补独立音频和双向连接执行、取消、计费、回执及断连恢复。 | [旧入口][old-relay]、[新入口][new-model] |
| F04 | P1 · 视频、MJ、Suno 异步任务 | 视频生成/查询/内容/remix、Kling/Jimeng、MJ、Suno 没有等价公共任务闭环。 | 分别补提交、轮询/回调、结果、原操作恢复和唯一扣费；现有图片任务不能覆盖所有任务族。 | [旧视频][old-video]、[旧 MJ/Suno][old-relay] |
| F05 | P1 · 助手流式与自定义 preset | agent 开启后仍 `stream_enabled:false`，Core 上游固定 `stream:false`，浏览器收到单次 JSON；只允许 builtin preset。旧版有真实 SSE 与 custom-v1。 | 恢复助手流式体验、取消/恢复及自定义 preset 的服务端策略与配置生效。多轮 agent 和工具执行已有。 | [新助手][assistant-web]、[新执行][assistant-execution]、[旧流式][old-assistant-stream]、[旧 preset][old-assistant-preset] |
| F06 | P1 · 渠道高级运维 | tag/multi-key/Codex 旧操作有明确501；`/test` 仅模型目录探测、`inference_tested=false`；手动激活密钥不等价旧随机/轮询池。 | 补所需高级操作、自动 key 选择/失效/恢复和真实推理测试；保留已有 native CRUD/价格发布。 | [新渠道][channels]、[观测语义][channel-observations]、[旧 key 池][old-channel-model] |

### 身份、订阅与支付

| 编号 | 优先级 / 工作包 | 当前差距 | 关闭条件 | 证据 |
| --- | --- | --- | --- | --- |
| F07 | P1 · OIDC / WeChat | 通用 OIDC 登录、WeChat 登录/绑定、旧 OIDC provider discovery/jwks/userinfo 等未对齐；新 status 无条件声明相关开关为 false。 | 恢复所需 provider、绑定/撤销、claims 与 issuer/client 校验和 UI。已有 OAuth2 authorization server 是另一合同。 | [状态][public-status]、[旧 OIDC][old-oidc]、[旧 API][old-api] |
| F08 | P1 · 外部支付订阅 | 新订阅只声明 `payment_methods:[wallet]`、`automatic_renewal:false`；旧 EPay 一次性套餐购买，以及 Stripe/Creem/Pancake 首购、周期与生命周期未齐。 | 按各 provider 原本支持的范围补产品快照、首购/周期 grant、失败、取消及原交易恢复；钱包充值不能抵扣订阅。 | [订阅入口][subscription-http]、[支付矩阵][payment-parity] |
| F09 | P1 · 旧订阅管理与重置 | reset voucher、用户订阅绑定/失效/删除、管理记录、root 批量重置未完整迁移。计划 CRUD、钱包购买/续期/取消/未用退款已有。 | 补具体管理行为的特权、审计、并发与额度语义，接通仍调用旧 API 的页面。 | [旧管理入口][old-api]、[新订阅][subscription-http]、[Web API][web-subscriptions] |
| F10 | P1 · 商店四类外部收款 | 当前兼容订单只接受 balance/free；旧 external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo 未接通。 | 区分卖家与平台收款，补原订单/交易/币种、库存交付、卖家结算与恢复；保留 Linux DO 的 LDC 单位。 | [订单限制][store-orders]、[商品限制][store-catalog]、[旧矩阵][payment-parity] |
| F11 | P1 · Pancake 与外部商店退款 | Pancake 退款 adapter unsupported；商店兼容退款 `supports_provider_sync:false`。Stripe、Waffo Global、Core ReleaseRefund 已有实现。 | 补原交易退款冻结/发送/查询/通知/失败释放与对账，补卖家结算回冲。通用 EPay/Creem 按旧合同处理；Coinbase 是新增能力，不计旧版回退。 | [Pancake][pancake]、[商店退款][store-refunds]、[旧矩阵][payment-parity] |
| F12 | P2 · Pancake 管理和 portal | catalog/pair/save、订阅商品映射/选项和 customer portal 尚无完整新后台；Web 保留调用。 | 接当前配置/权限、秘密只写、server 绑定 customer/store/environment，并做真实管理 UI 联动。 | [Pancake Web API][web-pancake]、[订阅 Web API][web-subscriptions]、[旧合同][payment-parity] |

### 业务后台与页面

| 编号 | 优先级 / 工作包 | 当前差距 | 关闭条件 | 证据 |
| --- | --- | --- | --- | --- |
| F13 | P2 · 脚本仓库管理 | GET/PUT repository 与 POST pull 固定501 `SCRIPT_REPOSITORY_UNSUPPORTED`；页面仍查询、保存和拉取。 | 补仓库配置/读取/拉取与权限、错误处理；普通脚本列表/编辑不能抵扣仓库功能。 | [operations][operations]、[脚本 UI][web-scripts] |
| F14 | P2 · 用量分享与数据源 | 非aggregate SVG返回503 `PROFILE_SHARE_USAGE_UNAVAILABLE`，UI仍提供models/profile/badge；aggregate LMM自动源禁用、live Cursor未移植。 | 接现有Core用量与所承诺的数据源/布局，验证公开字段、权限、更新和撤销；手填snapshot不等价自动同步。 | [分享实现][profile-share]、[分享 UI][web-share] |
| F15 | P1 · HeroSMS 当前业务 | 邮箱/SMS目录、报价、购买、取消、重下单、当前订单与配置未迁移；已有历史只读明确 supplier_operations_available:false。 | 补真实供应商IO、Core资金、取消退款和页面接线；保留已完成历史读取。 | [旧 API][old-api]、[新历史边界][hero-history]、[SMS Web API][web-sms] |
| F16 | P2 · io.net 模型部署管理 | 旧设置/连接、创建、列表、容器、日志、续期等未接新实现；models管理页仍调用旧 API。 | 恢复远端模型/容器部署业务及权限、供应商操作结果与失败恢复；这不是本仓库的安装/发布工具。 | [旧部署实现][old-deployments]、[Web models API][web-models] |
| F17 | P1 · 平台充值折扣码 | 管理、validate及新消费合同未对齐；Core明确没有discount-code authority。商店优惠券和充值兑换码均是不同业务。 | 补范围、用户/金额核验、次数/并发、消费绑定和管理 UI；保留旧折扣消费的迁移义务。 | [明确边界][discount-boundary]、[Web折扣码][web-discounts]、[旧 API][old-api] |
| F18 | P1 · 公开安全报告与Moderation后台 | 公开policy/stats和审核模型/任务/统计等旧UI合同未闭合。当前native abuse策略、标记、审计已有，不能称安全系统全无。 | 逐项映射native abuse，补旧审核队列/统计与操作并联调页面。其他申诉入口需继续逐行为确认。 | [新abuse][abuse]、[Web审核][web-security]、[旧审核实现][old-moderation] |
| F19 | P2 · RSS聚合 | Web仍请求/api/rss；旧版是有缓存、并发限制和SSRF保护的真实HTTP聚合，新后端未找到等价实现。 | 恢复feed读取、公开字段、缓存、错误和请求边界，验证真实页面。 | [旧RSS][old-rss]、[Web RSS][web-rss] |
| F20 | P2 · 价格变更webhook投递 | 站内ratio feed已有；旧签名HTTPS webhook、投递状态和retry后台未迁移。 | 补接收人/outbox、签名发送、持久投递/重试幂等和管理；不泛称旧邮件功能也已证实。 | [旧webhook worker][old-ratio]、[新站内通知][notifications] |
| F21 | P2 · 实时美元汇率查询助手 | 系统设置仍请求/api/option/exchange-rate，旧版真实查询外部汇率，新options没有对应路由。 | 补可信读取、校验/失败状态与UI；不重算历史账本，也不等于整体价格或换汇政策缺失。 | [旧汇率][old-fx]、[新options][options]、[Web设置][web-settings] |
| F22 | P2 · 市场AI自动审核 | 旧商品/工具市场有off/assist/auto及自动publish/reject；新只有人工审核，Web仍请求settings/ai-reviews。 | 补自动审核配置、执行/结果历史与权限边界；保留已有人工审核和版本发布流程。 | [旧自动审核][old-market-review]、[新人工审核][tool-review]、[Web审核设置][web-market-review] |

### 商店旧业务细节

| 编号 | 优先级 / 工作包 | 当前差距 | 关闭条件 | 证据 |
| --- | --- | --- | --- | --- |
| F23 | P1 · 游客购买与邮件提货 | purchase_login_required=false、email_pickup_link=true、pickup_login_required=false、pickup_email 被拒绝；登录买家的提货链接/提货码已有。 | 补游客身份/订单归属、支付对账/领取、邮箱验证、邮件outbox和重试；覆盖凭证混用、重放和退款后禁止领取。 | [当前商品边界][store-catalog]、[当前订单][store-orders]、[当前提货][store-pickup]、[旧游客入口][old-store-access]、[旧邮件worker][old-store-email] |
| F24 | P1 · 按数量/金额部分退款 | 旧版支持full/quantity/amount；新接口明确supports_quantity=false、supports_amount=false，原生退款也只处理整单本金。 | Core与Go共同绑定每次部分退款身份和原订单；累计上限、库存/交付、卖家收入、外部回执及丢ACK正确。 | [当前退款][store-refunds]、[原生资金校验][store-money]、[旧退款模式][old-store-refunds] |
| F25 | P1 · 商品分级可见性 | 旧public/registered/private三态未对齐；新拒绝非public、store_access_supported=false，但已有TestMode隐藏、卖家草稿与预览。 | 补三态查看/购买规则，在分页计数前按身份过滤；完成旧visibility/TestMode迁移，避免降为公开。 | [当前商品边界][store-catalog]、[旧访问模型][old-store-access-model] |
| F26 | P2 · 账户购物车与收藏 | store_collections_supported=false，缺服务端cart/favorites；点赞已实现。Web本地游客购物车不能代替账户持久化，当前能力关闭还会隐藏入口。 | 补账户隔离的持久化、清理和Web；验证下架/私有化/变体失效、跨账号及游客合并策略，再开放能力。 | [当前能力声明][store-catalog]、[Web动作][store-collection-actions]、[旧collections][old-store-collections] |

F15–F21 七组对应旧版 **65 个方法与路径声明**，含别名，仅用于后续回归矩阵；它不代表65个独立产品，更不是本次观察到65次线上失败。检查已核对旧控制器/服务真实调用及新原生替代，未把旧字符串消失单独当缺口。具体公网行为还受部署路由和配置影响，本次未逐条线上探测。[旧路由][old-api]、[Core fallback][http-fallback]

### 需要从旧欠账中扣除或分开的事项

- **商店优惠券、累计限购、登录买家提货码/链接已经实现。** 当前购买事务有优惠券核验与折后价格，累计限制包含pending订单；它们留在联合验收范围，不再按空缺功能计数。[优惠券][store-promotions]、[累计限购][store-limits]、[提货][store-pickup]
- **Gemini per-query 计价不算旧版回退。** 旧版只把非空 WebSearchQueries 变为bool，再固定按一次GoogleSearch收费；新 GroundedRequestV1 也是按请求。更细的按query合同属于扩展目标或待确认需求。[旧计价][old-grounding]、[新合同][grounding]
- 旧files、fine-tunes、images/variations和模型删除本来是RelayNotImplemented；部分native voice WebRTC/client-secret原本Unsupported，不计作迁移丢失。[旧模型路由][old-relay]
- FastPay按固定需求淘汰；通用EPay没有统一退款协议，Creem旧边界不等于自动退款，Waffo Global旧版没有订阅checkout；Coinbase是新增能力。[支付范围][payment-parity]
- 保留UNKNOWN/冻结款、拒绝未定义usage/价格是正确性边界。补能力需要定义合同和验证，不能取消校验来冒充完成。
- 新旧Playground页面都转到助手，`/pg` API兼容与页面入口是两个问题；本轮未将仅有旧路径差异额外计成整项产品缺失。其他未完整追到行为的旧管理接口保留逐项核查，不作为已证实缺口扩充数字。[历史功能审计][prior-parity]

## 5. N01：CLI 完整产品目标

锁定CLI明确是开发预览。有预设查询、catalog/status/doctor、桌面PKCE登录、模型/余额和退出；Linux/Pi1.2.0有受限Provider添加/恢复事务。其他软件的安装、接入、同步、更新、恢复、解除、卸载与运行仍返回阻塞原因；56项预设不是56个已接入服务商。[CLI README][cli]、[对照说明][cli-parity]

N01需要逐目标软件/版本完成真实接入与可恢复写入、各平台凭据/失败路径、完整命令行为、签名安装/更新与实际平台验收。它是新产品承诺，单列，不混入26组旧服务端能力差距。

## 6. 九组迁移与验收工作

这些项目通常已有代码、工具或部分历史实测，当前缺的是要求范围内、输入可追溯的证据闭环，不能一概说成“从未测试”。

| 编号 | 责任域 | 待关闭结果 | 最小关闭证据 |
| --- | --- | --- | --- |
| V01 | A01/A02 | 旧身份、特权、团队/开发者、密码/会话/Key/OAuth/MFA/Passkey 的迁移与激活 | 同一来源/隔离目标的映射；实际登录/撤销；deny 优先、跨租户、并发批准、旧凭证处置和全状态回读 |
| V02 | A03 | 钱包、欠款、返佣、订阅、Key 限额、预留和持续权益完整迁移 | 有来源依据的单位；逐笔/逐账户对账；账本平衡和应用余额；未决原操作不重复扣款 |
| V03 | A04 | 旧日志、用量、统计和财务导出正确读取 | 实际来源/目标范围、时区、分页、筛选、unknown 金额、权限、导出字段对照，不能只比计划文件 |
| V04 | A05/A06 | 支付、商店、客服、运营和商业资金的最终真实宿主闭环 | 当前 Core + 实际 Go main + PG；重复/乱序、撤权、并发消费退款、丢响应、双进程重启和配置变化后的旧订单；合成/外部 provider 分开记录 |
| V05 | A07/A08 | 模型矩阵、助手/MCP/工具/赞助付款方和 HumanGrant 安全委托 | 每个所需协议的 JSON/SSE/usage/取消/重试/价格；真实对话；跨租户/撤权/额度反例、恶意工具输出与原操作恢复 |
| V06 | A10 | 全站真实页面/操作、CLI 整合、近期 OAuth/广告/Extore 浏览器闭环 | 按角色/语言/桌面与窄屏/错误状态执行；截图、请求、数据库回读绑定候选；管理弹窗实际创建；Extore 授权/多规格/重复同步/撤销/隔离 |
| V07 | A09 | 当前 schema 的安装、已有候选升级、备份恢复与切换演练 | owner/runtime 分离、普通启动无 DDL、低权限拒绝；实际隔离恢复；writer fence/drain、死 holder CAS、有效写入保全和恢复后业务重启 |
| V08 | A07/A09 | 资源目标与相对旧版的性能/稳定性改善 | 同 CPU/内存、数据规模、功能与计费语义的优化构建对照；完整成功率、吞吐、P95/P99、RSS/PSS、长稳、慢消费者及故障恢复；整机与产品 cgroup 分开 |
| V09 | S11 | 同一源码/产物/配置/来源/目标的最终联合签收及质量债务关闭 | 各域固定输入、独立复验 owner、联合业务和故障证据；将 QD-001 退出条件逐条映射到真实记录；候选输入无漂移 |

以上来自 [PRODUCTION_ACCEPTANCE][acceptance] 和 [COMPLETION_SCOPE][completion]。V04–V06 应共享冻结候选与证据标识，分别证明后端事实、权限/供应商行为和实际用户旅程，避免只堆互不绑定的测试报告。

### 已有的历史实测应保留

- **钱包。** 历史记录包含普通原生用户的兑换、签到、红包、订阅购买/续期/退款、原请求丢响应恢复和 Core 进程重启。首次 owner 的余额识别已修复；管理弹窗修正有组件测试、typecheck 和构建，实际浏览器创建/回读待补。[集成记录][integration]
- **退款。** 现有进程测试覆盖真实 Core main、Go payment module worker、合成可计数 TCP provider；它明确不是 Go main 全量验收，也不是真实支付商通过。[退款进程测试][refund-process]
- **资源与慢消费者。** 记录包含 Core + Go 共享 1CPU/512MiB 的 240 秒和 540 秒轮次，分别完成 900/2100 个模型响应、180/480 笔退款，后者有 3348 个 UNKNOWN 保留项；双进程重启/原键重放不增加上游调用。后续联合轮次由开发者执行，独立测试者从冻结输入和原始记录重建核对，不能写成独立重新执行。[质量记录][quality]
- **OAuth/广告/Extore。** OAuth 已有隔离 Core HTTP Allow/Deny、code/token 旅程，完整 Chromium 未结束；广告丢响应为开发自测；Extore 的连接、同步、只读 UI 已补，仍待真实两端联调。[父状态][status]、[Extore][extore]
- **恢复。** runner 能执行初始化、ACL、加密备份和隔离恢复，但报告明确排除在线 writer fence、死 holder CAS、生产恢复、业务进程重启。[恢复 runner][recovery]

旧 OAuth Consent CSP FAIL 的证据缺少冻结二进制身份；Nov7 审计已纠正归因。当前有精确 callback 的 CSP 实现，应列新冻结浏览器复验，不能继续写成已确认 CSP 缺陷。[Nov7 审计][prior-report]

历史证据绑定自己的源码/产物和范围，不能自动变成当前 53c945dc 全站通过。现有受限产品 cgroup 测试也不等于整机 1C1G 通过，更不能替代新旧同负载性能比较。[资源和完整验收要求][acceptance]

## 7. 三组工程交付整改

### D01：权限边界审查与门槛恢复

本次实际执行 `python3 -B scripts/test-core-boundaries.py`，结果 **32 项、26 PASS、6 FAIL、exit 1**。六次失败涉及五类断言；两个测试复用同一个连接池计数 helper。它们不是六个已证实的漏洞，也不表示只需改六处：多个检查在首个指纹/计数断言处中断，后续断言尚未执行。

| 失败范围 | 实际原因 | 关闭方式 |
| --- | --- | --- |
| composition/operations 两项 | `openStorage` 预期3、实际5，新增 community/acquisition；Close 断言仍预期4、实际6 | 审查池归属、隔离、异常回收和关闭后扩展合同/反例 |
| storage authority | `composition/storage.go` 的 token hash 漂移 | 审查表权限正文增量，再更新已审查指纹 |
| bundle policy | `composition/storage_bundle.go` token hash 漂移 | 审查模块选择、namespace、权限上限和拒绝反例 |
| offline installer | `freshinstall/install.go` token hash 漂移 | 审查 owner schema/GRANT、目标限制、runtime 隔离及离线入口 |
| Core schema 清单 | 漏 `core_directory_ads.`、`core_referrals.` | 补目录及实际权限/未来 schema 拒绝反例 |

三个 hash 计算忽略注释和格式，变化涉及权限相关正文。应先审查实现，再同步合同和反例，不能直接换 hash 消除失败。通用 `core_*` SQL 拒绝与连接时的 `core_%` schema 拒绝仍存在，故本次报错尚未证明 Go 已取得 Core 权限。[检查源码][boundary]、[装配][composition-runtime]、[存储守卫][composition-storage]、[离线安装][freshinstall]

### D02：工作流绑定和示例格式

- 镜像流程要求仓库为 `TokenNotIncluded/lmm-api` 且为旧 WIP 分支或 preview tag；文件现在位于 `lmm-api-source`，仓库条件失配，源码获取也指旧 WIP。[镜像流程][images-workflow]
- S11 快照流程仍绑定旧仓库、旧 WIP、历史 PR；协议检查的 push/PR 分支也仍是旧 WIP。[快照流程][snapshot-workflow]、[协议流程][protocol-workflow]
- 分发流程检查 `/usr/share/lmm/legal/site.example.json`，实际 Dockerfile 安装 `site.example.toml`。[分发流程][distribution-workflow]、[Dockerfile][extensions-docker]

需维护真实使用的本地/CI 交付路径，修复引用和断言，验证当前仓库的对应流程。迁仓时 Actions 被有意关闭，因此“没有自动运行”本身不等于业务故障或没有本地验证。本次 10 个镜像 SourceContracts 测试通过，其中会验证现有旧 guard；这说明测试通过仍可能是在检查过时合同。[迁仓说明][migration]、[SourceContracts][image-tests]

### D03：制品、安装/发布证据与状态同步

本次 GitHub 查询时，源码仓库无正式 Release，父仓库最新正式 Release 仍为旧 Go `go-v0.2.118`。当前 source SHA 没有 check runs；源码仓库可见的单次 Actions 记录是其他分支的历史 Copilot review，不能作为本候选验收。需将经 V09 复验的 Core/Extensions/CLI/Web 制品、来源、平台、校验/签名、安装/升级记录关联，再形成交付。[旧 Release][legacy-release]、[发布规则][release-architecture]

至少有四类文档漂移：父 STATUS 引用 `eabb74e`，早于实际 gitlink；Extore UI 的旧状态落后当前代码；旧非支付清单仍列一些已实现模块；支付说明与测试 README 的部分描述落后 Waffo Global/ReleaseRefund 实现。旧 schema 数量也不能代替当前56项。本报告记录这些差异，不代表原文件已全部修复。[父状态][status]、[旧非支付清单][nonpayment]、[支付说明][payment-doc]、[schema][schema]

## 8. 本次实际验证与限制

| 检查 | 结果 | 能证明什么 |
| --- | --- | --- |
| `test-core-boundaries.py` | 32项；26 PASS / 6 FAIL；exit1 | 当前静态边界合同失配，见D01 |
| `test_core_cutover_preflight.py` | 19/19 PASS；exit0 | 预检合同逻辑，不是实际目标 READY |
| `test_build_release_manifest.py` | 4/4 PASS；exit0 | manifest 构造/一致性，不是真实发布 |
| `test_docker_image_publication.py SourceContracts -v` | 10/10 PASS；exit0 | 离线源码/脚本合同，不含 Docker/GHCR 发布 |
| `quality-gate.py full --base 53c945dc… --plan`，CLI 初始化后 | exit1；`checks=[]`；`expired debt/exception: QD-001` | **完整测试尚未开始**，计划被到期债务阻断 |
| 文档品牌和差异格式检查 | PASS | 报告文档规范，不是产品验收 |
| 公开 GET `https://api.lmm.best/api/status` | 本环境代理/网关 HTTP502，未取得版本JSON | 当前线上版本无法核实，不能据此断言宕机或运行某一版本 |

完整计划最初报告 CLI 未初始化；已按 gitlink 初始化到 `f86ec5e` 后重试，当前有效阻断是 QD-001，不再把 CLI 缺失当作项目问题。其他未初始化的开发技能/集成子模块不据此认定为产品缺失。

QD-001 是 open/critical，期限2026-10-24，reviewer pending、evidence为空。退出条件是完整真实 host/funds 的独立权限、钱包/账本、重复、并发、失败与流取消证据。应逐条映射历史记录、补缺并独立复验，不能通过延后期限、降等级或手填记录代替关闭。[债务原文][debt]

执行环境 Linux、Python3.12.14；质量计划记录4954个输入文件项。本次产品源码未更改，未绕过债务执行 full 测试。当前公开版本头规则只要求 `X-LMM-API-Version`；源码/产物/schema/route完整身份在私有切换记录验证，缺少这些公开头不是缺陷。[发布身份规则][release-architecture]

## 9. A01–A10 与 S11 总表

| 责任域 | 当前判断 | 剩余归属 |
| --- | --- | --- |
| A01 权限 | 核心权限/隔离已有；完整角色和真实特权迁移证据待闭合 | V01、D01 |
| A02 身份 | 多种登录、会话、MFA/Passkey已有；OIDC/WeChat及真实迁移未齐 | F07、V01 |
| A03 资金 | 账本、余额、订阅、活动已有；真实来源权益与最终目标对账待完成 | F08–F11、F24、V02 |
| A04 历史 | 读模型/导出已有；真实范围、单位和权限验证未闭合 | V03 |
| A05 支付 | 钱包多provider和部分退款恢复已有；外部订阅、Pancake/商店退款仍欠 | F08、F10–F12、V04 |
| A06 商业扩展 | 多模块和余额商店已有；旧业务及界面断口仍在 | F10–F26、V04、V06 |
| A07 模型 | 文本/Input API、流式、路由与计费骨干已有；完整协议媒体范围不足 | F01–F04、F06、V05、V08 |
| A08 助手/工具 | 多轮、持久化与工具执行已有；助手流式、自定义preset、AI审核仍欠 | F05、F22、V05 |
| A09 安装/恢复 | 工具、预检和部分恢复/资源证据已有；最终升级切换、新旧对照未闭合 | V07–V08、D01–D03 |
| A10 Web/CLI | 大量页面及部分真实旅程已有；全站矩阵和CLI产品承诺未齐 | 对应F项、N01、V06 |
| S11 联合集成 | 当前最终组合尚不能签收 | V09，依赖全部必需项 |

“待闭合”表示当前证据不足以签收整域，不表示该域没有开发或测试。

## 10. 推荐推进顺序

1. **恢复可信的质量与交付反馈。** 先做D01权限正文审查和反例，修D02错仓库/分支/示例检查；将QD-001已有证据逐条映射，明确还需补的真实宿主与资金测试。
2. **按领域推进功能包。** 模型/渠道、身份/订阅、支付/商店、助手/审核、业务后台可以并行认领；每个包以本表关闭条件为准，接口变化同步调用方。CLI另按目标软件推进。
3. **同步准备迁移和真实用户旅程。** 用同一授权来源/隔离目标进行身份、资金、历史迁移；及时核对单位、权限和页面实际操作，避免到最后才发现业务合同不兼容。
4. **冻结候选并完成联合签收。** 同一Core/Go/Web/CLI制品完成全站矩阵、故障恢复、资源和新旧性能对照，再形成可安装交付和实际目标的切换证据。

本次不估人日：实时协议、外部支付明显大于RSS或示例修正，各包还共享实现和验收。真实旧数据、供应商测试环境、CLI目标版本也影响工期。应先为这些工作指定负责人并完成一轮可运行切片，再用实际交付速度估日期。

最近仓库状态仍称生产没有切换；本次未取得绑定当前源码、制品、配置和生产进程的激活证据。公开status在本环境得到502，不能判断线上运行旧版还是新版，更不能直接断言生产宕机。[最近状态][status]、[激活模板][activation]

## 附：报告复核原则

按五方面复核：直接证据是否支持结论；新旧基线是否区分；实现/模拟/真实进程/外部服务/生产是否分开；每个欠账是否有关闭条件；已完成进展、旧失败和未执行范围是否保留。未用文件/路由数推完成率，未把受控拒绝写成漏洞，未通过缩小既定范围消除欠账。

[source]: https://github.com/TokenNotIncluded/lmm-api-source/tree/53c945dccb3ef1a105261bc6a91c01dfc6f26191
[recipes]: https://github.com/TokenNotIncluded/lmm-api/tree/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6
[legacy-release]: https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118
[prior-report]: https://docs.google.com/document/d/1ONq6Fl-vnSpzqNdOV2PJwMQSfR2XYLbCN2sj00YrQLw/edit?usp=drivesdk
[prior-parity]: https://drive.google.com/file/d/18DExsWq7YTTCv_C5Vu7VHxV6w0oeWLkb/view?usp=drivesdk
[acceptance]: https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md
[completion]: https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-api/COMPLETION_SCOPE.md
[payment-parity]: https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-api/PAYMENT_PARITY.md
[status]: https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/STATUS.md
[migration]: https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/MIGRATION.md
[cli]: https://github.com/TokenNotIncluded/lmm/blob/f86ec5e605b00946a7227db78519c482476365e3/README.md
[cli-parity]: https://github.com/TokenNotIncluded/lmm/blob/f86ec5e605b00946a7227db78519c482476365e3/docs/magpie-parity.md
[new-model]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/model_http.rs#L383-L410
[input-api]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/model_http/input_api.rs
[request-parser]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/relay/request.rs#L421-L451
[response-parser]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/relay/request.rs#L615-L644
[anthropic]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/relay/native_anthropic.rs
[gemini]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/relay/native_gemini.rs
[channels]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/channel_http.rs#L60-L177
[channel-observations]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/channel_http/control/observations.rs
[grounding]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/billing/grounding_price.rs#L5-L38
[assistant-web]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/webcompat/assistant.go
[assistant-execution]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/model_http/assistant_execution.rs#L180-L190
[passkeys]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/identity/browser/passkeys.rs
[promotions]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/promotions.rs#L293-L323
[subscriptions]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/subscriptions.rs#L433-L484
[subscription-http]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/subscriptions/http.rs#L39-L87
[affiliate]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/affiliate.rs
[public-status]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/http.rs#L233-L255
[payment-checkout]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/webcompat/payment_checkout.go
[waffo-refunds]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/payments/waffo_refunds.go
[refund-rpc]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/payments/rpc.rs#L317-L323
[pancake]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/payments/pancake.go
[store-orders]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/legacy_orders.go#L203-L266
[store-catalog]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/legacy_catalog.go
[store-refunds]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/legacy_refunds.go
[store-pickup]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/pickup.go
[store-promotions]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/promotions.go
[store-limits]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/purchase_limits.go
[store-money]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/store/money_validation.go
[store-collection-actions]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/store/collection-actions.tsx
[web-subscriptions]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/subscriptions/api.ts
[web-pancake]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/system-settings/integrations/waffo-pancake-api.ts
[operations]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/webcompat/operations.go
[web-scripts]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/scripts/scripts-panel.tsx
[profile-share]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/community/profile_share.go
[web-share]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/profile/share-page.tsx
[hero-history]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/history_read/hero_sms.rs#L130-L142
[web-sms]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/email-activations/sms-api.ts
[web-models]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/models/api.ts
[discount-boundary]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/promotions/red_packets.rs#L223-L237
[web-discounts]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/discount-codes/api.ts
[web-security]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/system-settings/security/security-audit-api.ts
[abuse]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/abuse/admin.rs#L549-L569
[web-rss]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/rss/api.ts
[notifications]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/console_compat/notifications.rs
[options]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/options_http.rs
[web-settings]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/system-settings/api.ts
[tool-review]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/toolmarket/web.go#L427-L457
[web-market-review]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/features/market-ai-review/api.ts#L54-L66
[http-fallback]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/http.rs#L165-L196
[schema]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/core_schema.rs
[extensions-run]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/app/run.go
[extore]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/extore-shop-binding-followup.md
[integration]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/integration-verification.md
[refund-process]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/tests/payments_refund_process.rs
[quality]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/quality.md
[recovery]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/tests/architecture_completion/run_core_recovery.py
[boundary]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/scripts/test-core-boundaries.py
[debt]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.quality/debt.json
[composition-runtime]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/composition/runtime.go
[composition-storage]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/composition/storage.go
[freshinstall]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/freshinstall/install.go
[images-workflow]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/microservice-images.yml
[snapshot-workflow]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/s11-source-snapshot.yml
[protocol-workflow]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/core-protocol.yml
[distribution-workflow]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/microservice-distribution.yml
[extensions-docker]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/deployment/docker/extensions.Dockerfile
[image-tests]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/scripts/test_docker_image_publication.py
[release-architecture]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/release-architecture.md
[nonpayment]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/nonpayment-completion.md
[payment-doc]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/payment-gateways.md
[activation]: https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/deployment/native/templates/activation-record.example.json
[old-api]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/api-router.go
[old-relay]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/relay-router.go
[old-video]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/video-router.go
[old-channel-model]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/model/channel.go#L283-L351
[old-oidc]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/oidc_provider.go
[old-assistant-stream]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/controller/assistant_agent_loop.go#L270-L277
[old-assistant-preset]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/model/assistant_prompt_preset.go#L319-L341
[old-grounding]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/service/text_quota.go#L210-L219
[old-deployments]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/controller/deployment.go#L494-L518
[old-moderation]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/controller/security_moderation.go#L20-L95
[old-rss]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/controller/rss.go#L114-L245
[old-ratio]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/service/ratio_notification.go#L65-L169
[old-fx]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/controller/exchange_rate.go#L64-L151
[old-market-review]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/model/market_ai_review.go#L436-L489
[old-store-access]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/merchant_store_access.go
[old-store-email]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/service/merchant_store_email.go
[old-store-refunds]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/model/merchant_store_refunds.go
[old-store-access-model]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/model/merchant_store_access.go
[old-store-collections]: https://github.com/TokenNotIncluded/lmm-api/blob/cc52aa195c6f2a01d703a1a407be73c31b89867d/apps/api-go/router/merchant_store_collections.go
[web-routes]: https://github.com/TokenNotIncluded/lmm-api-source/tree/53c945dccb3ef1a105261bc6a91c01dfc6f26191/apps/web/src/routes

# 新架构进度

这是源码仓库与配方仓库共用的唯一进度列表。2026-11-08 重新建立，按产品行为与验收结果分组；旧任务编号、旧勾选状态和历史完成率不再沿用。架构规则见[核心与扩展](core-migration.md)，完整验收范围见[生产验收合同][acceptance]。

## 基线与使用方法

本表依据产品源码 [53c945dc][source]、旧稳定版 [go-v0.2.118][legacy] 和 [2026-11-08 固定审计][audit]。本次文档整理没有更改产品实现，也没有重新执行业务验收。后续实现提交须逐项补充自己的源码、产物与验证记录；不能把文档提交当作产品进展。

| 状态 | 含义 |
| --- | --- |
| 待补齐 | 已确认存在功能或行为差距，需实现并验证 |
| 待修复 | 已复现的工程合同或交付配置问题 |
| 待验收 | 已有实现、工具或历史实测，当前候选的完整证据未闭合 |
| 待前置 | 依赖实现、独立验收或交付输入先完成 |
| 已验收 | 本项范围由固定输入及独立证据证明；注明日期与验收者 |

每项以表中关闭条件为准。开始实施时记录负责人与候选提交；申请关闭时记录实际命令、结果、原始证据位置和独立验收者。失败、未运行和范围外行为分别说明。当前列出 **39 项推进事项**，大小与依赖不同，不用项目数换算完成率或工时。尚未验收不等于从未实现或从未测试。

## 首先解除的阻断

审计基线的边界检查为 32 项、26 项通过、6 项失败；完整质量门禁在计划阶段被过期且仍为 open/critical 的 QD-001 阻断，`checks=[]`。这些原始记录保留在[固定审计][audit]，本轮修复与独立复验见下表及[原始证据][rel-evidence]。

REL-01 的静态边界阻断已解除；REL-02 的配置修复已提交，剩真实镜像资源与启动验收。CHECK-09 仍需逐条补齐 QD-001 退出证据；不得通过只改指纹、延后期限、降级债务或手填通过记录消除阻断。

2026-11-08 固定候选为 [7fb02902][rel-candidate]，基于 `bb2697e9216d67bf1f34705af570ed2d8eb5288d`；开发分别由 `rel01_boundaries`、`rel02_delivery` 负责，`rel_acceptance` 独立审查并执行检查，验证前后工作树干净。真实 PostgreSQL、容器及资金业务验收未运行。

## 工程与交付

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| REL-01 | 静态边界合同已对齐：模块连接池、4 处已审查正文指纹及 Core schema 拒绝范围 | 已验收 | 2026-11-08，`rel_acceptance` 在 [7fb02902][rel-candidate] 独立核对池归属/关闭、GRANT、namespace 与安装权限，33 项边界检查通过；补齐 directory_ads/referrals 及拒绝反例。[原始证据][rel-evidence]。范围为静态源码合同，真实数据库权限验收仍按 CHECK 项执行。 |
| REL-02 | 当前源码/main、TOML 与固定来源已对齐；真实镜像验收待补 | 待验收 | 修复 [15bc5748][rel02-fix]；独立拓扑 15、镜像合同/模拟编排 16、归档 fixture 14 项及 8 个工作流语法检查通过。还需用固定候选实际构建 Extensions runtime 镜像，执行镜像内资源检查和同镜像离线非 root 启动；当前环境无 Docker。[原始证据][rel-evidence]。Actions 启用仍须另行决定。 |
| REL-03 | 当前产品组合的制品、安装升级与发布证据未闭合 | 待前置 | CHECK-09 关闭后，关联 Core、Extensions、CLI、Web 的来源、平台、摘要/签名、安装升级记录与 Release；发布和生产切换分别记录。 |

## 身份、模型与助手

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| AUTH-01 | 通用 OIDC、WeChat 登录/绑定及旧 OIDC provider 合同未对齐 | 待补齐 | 补 provider、claims、issuer/client 校验、绑定与撤销和实际页面。保留已实现的 OAuth2 服务端、MFA 与 Passkey，分别验收。 |
| MODEL-01 | Chat/Responses 视觉输入、Anthropic image/cache_control/hosted tools、Gemini inlineData/fileData 等未齐 | 待补齐 | 补解析、协议转换、usage、价格与恢复；验证合法输入、非法输入、实际响应。独立图片生成不替代聊天视觉。 |
| MODEL-02 | Responses 持久化/previous_response_id 与部分旧同步 API 缺等价行为 | 待补齐 | 补存储、续接、工具、重放与计费；逐项恢复所需 completions、compact、search、decisions、moderations 等实际行为。 |
| MODEL-03 | 独立 TTS/STT/translation 和双向 Realtime/WebSocket 未齐 | 待补齐 | 补连接、取消、回执、计费与断连恢复；当前 Chat audio 与模型 SSE 继续保留，不能据此关闭本项。 |
| MODEL-04 | 视频、Kling/Jimeng、MJ、Suno 异步任务未形成完整闭环 | 待补齐 | 按任务族补提交、查询/回调、结果/remix、原操作恢复与唯一扣费。 |
| MODEL-05 | 渠道 tag、多 key 轮换、Codex 运维和真实推理探测未齐 | 待补齐 | 补自动选择/失效/恢复与真实调用测试；现有渠道 CRUD、密钥库、价格发布和目录探测保留。 |
| ASSIST-01 | 助手仍返回单次 JSON，缺真实 SSE 与自定义 preset | 待补齐 | 接通上游流式与浏览器流式、取消/恢复；补自定义 preset 的服务端策略和配置生效。多轮 agent 与工具执行已有。 |

## 订阅、支付与商店

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| SUB-01 | 外部支付订阅缺口：EPay 一次购买及 Stripe/Creem/Pancake 的原支持生命周期 | 待补齐 | 按 provider 补产品快照、首购/周期 grant、失败、取消和原交易恢复；钱包充值与余额订阅分别验收。 |
| SUB-02 | 订阅 reset voucher、绑定/失效/删除、管理记录与批量重置未齐 | 待补齐 | 补特权、审计、并发和额度语义，接通仍调用旧接口的页面；已有计划 CRUD 与余额购买/续期/退款保留。 |
| PAY-01 | 商店仅接 balance/free，缺四类旧外部收款 | 待补齐 | 恢复 external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo；分别证明卖家/平台收款、币种、库存交付、结算和恢复，保留 LDC 单位。 |
| PAY-02 | Pancake 及外部商店退款未齐 | 待补齐 | 补原交易冻结、发送、查询/通知、失败释放、对账及卖家收入回冲；已有 Stripe/Waffo Global 退款与 Core ReleaseRefund 按现合同验收。 |
| PAY-03 | Pancake catalog/pair/save、订阅映射与 customer portal 未接完整后台 | 待补齐 | 接当前配置和权限、秘密只写、server 绑定 customer/store/environment，并完成真实管理页面联动。 |
| STORE-01 | 游客购买与邮件提货合同未齐 | 待补齐 | 补游客订单归属、邮箱验证、付款领取、邮件 outbox/重试；覆盖凭证混用、重放及退款后禁止领取。登录买家的提货码/链接已有。 |
| STORE-02 | 商店按数量/金额的部分退款缺失 | 待补齐 | Core 与 Go 绑定原订单和每次退款身份；证明累计上限、库存/交付、卖家收入、外部回执及丢 ACK 恢复。 |
| STORE-03 | 三态目录、详情与分页已补齐；卖家自购及管理页待完成 | 待补齐 | 2026-11-09，`commerce` 实现 [63db64bc](https://github.com/TokenNotIncluded/lmm-api-source/commit/63db64bc0124f0361f0f5a7facde72b0c904d46a)，`root` 独立复验真实 PostgreSQL 的三态可见性、分页前过滤、原生游标、伪造身份拒绝及旧 visibility/TestMode 优先级，两项通过。[原始记录][progress-evidence]。尚需补卖家自购的 Core 资金合同及页面操作，`store_access_supported` 保持关闭。 |
| STORE-04 | 账户购物车、收藏、清理和游客合并已接通；完整页面验收待补 | 待验收 | 2026-11-09，`commerce` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 PostgreSQL 的账户隔离、绝对数量并发重放、重启读回、私有化隐藏、暂停保留及下架/变体清理；既有库离线迁移与精确 DML 权限用例通过。已有页面按 `max(server,guest)` 合并后发送绝对数量 PUT；能力已开放，不预占资金或库存。冻结 Extensions 构建通过。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL，浏览器操作与最终组合按 CHECK-06 继续验收。 |
| REVIEW-01 | 市场 AI 自动审核缺配置、执行和结果历史 | 待补齐 | 恢复 off/assist/auto 与发布/拒绝边界，接审核设置和权限；保留现有人工审核与版本发布。 |

## 业务后台与页面

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| APP-01 | 脚本仓库读取、保存与 pull 固定返回 unsupported | 待补齐 | 补真实仓库配置与拉取、权限和错误状态，并验证管理页面；普通脚本编辑不覆盖仓库业务。 |
| APP-02 | 用量分享的非 aggregate SVG、LMM 自动数据源与 live Cursor 未齐 | 待补齐 | 接 Core 实际用量与所需来源/布局；验证公开字段、权限、更新与撤销，区分手填 snapshot。 |
| APP-03 | HeroSMS 当前目录、报价、购买、取消、重下单和管理缺失 | 待补齐 | 补真实供应商 IO、Core 资金、退款和页面接线；现有历史只读不变。 |
| APP-04 | io.net 远端模型部署管理未迁移 | 待补齐 | 补连接设置、创建/列表、容器/日志、续期、权限与失败恢复；与本项目安装发布工具分开验收。 |
| APP-05 | 平台充值折扣码缺管理、校验与新消费合同 | 待补齐 | 补范围、用户/金额核验、次数/并发、消费绑定及旧数据迁移。商店优惠券与充值兑换码已有，属于独立行为。 |
| APP-06 | 公开安全报告与 Moderation 后台未完成旧行为映射 | 待补齐 | 对照 native abuse，逐项补 policy/stats、审核队列/任务/统计和页面操作；已有策略、标记、审计继续保留。 |
| APP-07 | RSS 抓取、缓存、设置保存与页面接线已实现；线上 edge 与浏览器验收待补 | 待验收 | 2026-11-09，`business_apps` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 HTTP 抓取、RSS/Atom/RDF 投影、并发/缓存/错误与网络边界；三项真实 PostgreSQL 用例证明配置持久化、并发版本校验、v1→v2 离线升级及原操作恢复、HTTP 权限与禁用源不抓取。七项定向 Go race 测试、冻结 Extensions 构建及 34 项边界检查通过。配置仅授予 SELECT/UPDATE，普通启动不执行 DDL。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL；实际 Nginx 路由与浏览器验收仍需目标环境。 |
| APP-08 | 价格变更 webhook 投递与重试后台未迁移 | 待补齐 | 补订阅接收人、outbox、签名 HTTPS、持久状态与幂等重试；现有站内通知另行验收。 |
| APP-09 | 设置页美元汇率助手：补齐 Core 报价、展示汇率保存与页面接线 | 待补齐 | 2026-11-09 开始实施，源码基线 `4c7a29b9`；`app09_backend`、`app09_web` 分别负责实现，`app09_acceptance` 独立验收。补可信外部读取、校验、失败状态、手动保存及状态回读；保持历史账本与金额单位不变。 |

## 客户端

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| CLI-01 | 锁定 CLI 仍为开发预览，多软件与多平台接入未齐 | 待补齐 | 按目标软件/版本完成真实接入、可恢复写入、凭据与失败路径、安装/同步/更新/解除/卸载/运行和签名分发；预设目录数量不代表已接入数量。 |

## 迁移与独立验收

这些项目保留已有实现及历史实测的价值；关闭时需证明当前最终组合，不从旧报告自动继承通过。CHECK-04、CHECK-05、CHECK-06 共用冻结候选和证据标识，分别证明真实宿主、协议/权限与用户操作。

| 编号 | 验收目标 | 状态 | 关闭条件与最小证据 |
| --- | --- | --- | --- |
| CHECK-01 | 身份与权限迁移 | 待验收 | 同一来源/目标的身份、特权、团队/开发者、密码/会话/Key/OAuth/MFA/Passkey 映射；实际登录/撤销、deny 优先、跨租户和并发批准，旧凭证处置与全状态回读。 |
| CHECK-02 | 资金与持续权益迁移 | 待验收 | 钱包、欠款、返佣、订阅、Key 限额、预留与持续权益具备来源和单位；逐笔/逐账户对账、账本平衡，原操作恢复不重复扣款。 |
| CHECK-03 | 历史读取与导出 | 待验收 | 实际日志/用量/统计/财务的来源目标范围、时区、分页、筛选、unknown 金额、权限和导出字段逐项对照。 |
| CHECK-04 | 支付、商店、客服及商业资金闭环 | 待验收 | 当前 Core + 实际 Go main + PostgreSQL；重复/乱序、撤权、并发消费退款、丢响应、双进程重启及配置变化后的旧订单。合成 provider 与真实支付商分别报告。 |
| CHECK-05 | 模型、助手、MCP、工具与 HumanGrant | 待验收 | 所需协议的 JSON/SSE/usage/取消/重试/冻结价格；真实对话、赞助付款方、跨租户/撤权/额度反例、恶意工具输出与原操作恢复。 |
| CHECK-06 | 全站页面、CLI 与跨仓库集成 | 待验收 | 按角色/语言/桌面与窄屏/错误状态执行，截图、请求与数据库回读绑定候选；实际管理弹窗、OAuth consent、广告与 Extore 两端授权/多规格/重复同步/撤销/隔离。Extore main 自动部署限制继续生效。 |
| CHECK-07 | 安装、升级、备份恢复与切换 | 待验收 | 当前 schema 的 owner/runtime 分离、普通启动无 DDL、低权限拒绝；实际隔离恢复、writer fence/drain、死 holder CAS、有效写入保全与业务重启。 |
| CHECK-08 | 资源、旧新版性能与稳定性 | 待验收 | 相同 CPU/内存、数据、功能和计费语义下的优化构建；完整成功率、吞吐、P95/P99、RSS/PSS、长稳、慢消费者和故障恢复；产品 cgroup 与整机预算分开。门槛见[质量指南](quality.md#独立业务与资源验收)。 |
| CHECK-09 | 最终冻结组合与 QD-001 关闭 | 待前置 | 前置实现、迁移和验收全部关联同一源码/产物/配置/来源/目标；不同人员/agent 独立复验，逐条满足 QD-001 的真实 host/funds 条件，保留失败记录并确认无输入漂移。 |

## 维护约定

- 产品范围和支付支持边界分别以[完整验收合同][acceptance]、[支付矩阵][payment-parity]为准；删掉旧任务文件不缩减需求。
- 专题指南解释接口、配置和操作，禁止复制本表的动态状态；变更进度只更新本文件。
- 关闭一项时，在该行附固定提交/产物、证据和日期；独立验收未完成时保持“待验收”。产品缺口解决不等于已发布、已上线或完成真实迁移。
- 旧审计、失败记录与任务提示词从[历史索引](archive.md)访问。历史内容只能证明其冻结输入和实际执行范围。
- 当前文档更新时间不代表线上版本；公开版本规则与私有切换身份见[发布说明](release-architecture.md)。

[source]: https://github.com/TokenNotIncluded/lmm-api-source/tree/53c945dccb3ef1a105261bc6a91c01dfc6f26191
[legacy]: https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118
[audit]: https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/architecture-parity-audit-2026-11-08.md
[acceptance]: https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md
[payment-parity]: https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md
[rel-candidate]: https://github.com/TokenNotIncluded/lmm-api-source/tree/7fb02902eefd21670840125c73bea539fab853ce
[rel02-fix]: https://github.com/TokenNotIncluded/lmm-api-source/commit/15bc57486442a3a2deae538ba7105069c6dd9c81
[rel-evidence]: https://drive.google.com/drive/folders/1c1MpzgGZ7yDk7F53b28aDeoOfx8h9zxy
[progress-evidence]: https://drive.google.com/drive/folders/1WpvJDD0IDjB8p-qr14k9_lgP9aWZ2hMw
[collections-rss]: https://github.com/TokenNotIncluded/lmm-api-source/commit/c45beffa4bb0aa4880f465788a6955708ae13ab6

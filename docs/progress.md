# 新架构进度

更新于 2026-11-12。此文件维护当前状态；历史执行过程和失败记录保留在 Git 历史与对应验收收据。

## 上线现状与剩余阻塞

- **尚未上线新架构**。公开服务最近核验为旧版 `0.2.118`，HTTP 200。
- **本地迁移演练**已完成身份、资金、商店/客服、订阅、OAuth/Passkey、用户组及信任基线导入读回。管理员正常激活已成功，Root auth_version从6升至7；42条密钥原版本与撤销状态保留，14条未撤销，另两个原禁用管理员保持禁用。旧raw/sk密钥共84次本地只读HTTP认证已通过，24次成功、60次按原生命周期拒绝，62张资金/计费表不变；使用合成模型目录，仅证明认证连续性。ed5fe19已修正式授权脚本owner参数化，自定义/default owner与错误owner拒绝均通过真实PG验证。
- **正式制品与固定组合**已验证。Core7 `e5b3e11`、Go9 `76aaf71`、Web5 `907197b` 的 HTTP15项、浏览器17项及release图片检查通过，采用隔离账号和本地供应商。Core7 Arch/DEB `0.1.19-7` 已构建及校验，当前提交与包未签名。
- **新依赖候选待验**。e2a64cd已将zconv升级至0.2.2、Rust对齐1.99.1；d8843d6修复空system数组生成额外消息的问题，视觉7项均通过。46adc05同步了mise、CI、原生打包、Docker构建及验收runner的Rust版本，25项针对性静态检查通过，官方Docker manifest摘要独立核对一致；当前候选的构建与完整组合验收继续进行，Core7证据保留原版本范围。
- **模型配置迁移尚未完成**。共同目标的渠道凭据引用、价格版本、路由配置及发布均未落地，active release 的 config_version 为空。Source有29个渠道、23个启用；用户组21199条已导入。已生成29个凭据引用及完整正常规划，但候选仍被阻塞；来源配置673个模型；实际30天消费82个，其中79个仍在配置，7天使用49个。当前在用79个中55个有上下文合同、73个有价格合同；正在补剩余精确合同及runtime/WebSocket/Anthropic映射，未用配置继续保留。渠道47后转换策略已编码，测试尚未通过确认。
- **生产仍需完成**最终停写捕获、全阶段迁移与对账、部署切流和实际业务验收。旧快照 `FINAL_20261111T120751Z` 已因旧服务恢复写入而失去未来切换资格。
- **历史查询迁移未完成**。正常history阶段预览已生成193423条日志、20474条用量聚合和115笔成功充值历史，尚待实际导入及独立读回；孤儿记录和未导性能指标保留原件与明确缺口。
- **历史付款责任仍在**。95笔原字段完整的充值依据已在本地安装读回；81笔缺历史字段、13笔ePay缺冻结付款快照、3笔孤儿继续保留未决权益和补证路径。60条历史订阅订单保持UNKNOWN，不能当作已自动结算。

## 首发范围

首发必须支持现有账号与密钥、文字和图片模型API、充值计费（含人民币）、商店购买/库存/提货/退款、订阅及既有数据权益。用户允许未使用功能和非关键问题延期，Gemini音频延期，zcall更新下版本处理。以下39项保留完整产品范围；未使用功能不自动成为首发阻塞，延期项继续跟踪。

## 验收状态核对

原文15项“待验收／待前置”已逐项重核。旧说明中的编译失败、仅旧合成源及无浏览器结果已按当前证据更新。状态为“部分验收”的项目仍有明确剩余范围；“本地演练通过”尚需最终生产迁移。

| 编号 | 当前状态 | 已完成部分与剩余工作 |
| --- | --- | --- |
| REL-01 | 历史范围已验收 | 静态边界合同已验收；当前运行权限见 CHECK-07。 |
| REL-02 | 历史范围已验收 | 旧候选镜像资源与启动已验收；当前原生制品见 REL-03。 |
| REL-03 | 部分完成 | Core7、Go9、Web5 制品与本地组合已验证；剩签名/发布、生产安装及运行核验，CLI 分发仍见 CLI-01。 |
| AUTH-01 | 实现待验收 | OIDC与WeChat验证码桥已有provider管理、登录绑定和签名校验实现；旧provider配置迁移等价、正式配置和浏览器全链仍待验。见 services/core/src/identity/federated/。 |
| MODEL-01 | 部分实现 | 文字、视觉输入及Anthropic缓存边界已有实现；hosted tools/fileData完整入口与转换覆盖仍待核实。 |
| MODEL-02 | 部分实现 | Responses WebSocket已接previous_response_id续接；持久化、HTTP查询删除及旧同步API等价性尚未证明。 |
| MODEL-03 | 部分实现 | 已有Chat音频计费与恢复；独立TTS/STT/translation及双向Realtime未齐。Gemini音频按用户决定延期。 |
| MODEL-04 | 待核实 | 视频、Kling/Jimeng、MJ、Suno完整异步闭环尚无充分证据；不据旧审计推断完全无实现。 |
| MODEL-05 | 部分实现 | 渠道目录、余额探测及原生tag发布/查询/分组分页已有；tag权限、版本冲突、原子发布与重建读回的真实PG针对性自测通过，当前冻结候选独立验收待执行。Web配置读回已保留target.client_protocols，避免编辑时丢失协议限制。多key轮询管理入口与真实推理探测待补；共同演练已生成29凭据引用，完整规划仍因价格/上下文/runtime及请求字段映射缺口阻塞，尚未发布。 |
| ASSIST-01 | 实现待验收 | Core/Go实时流、Web历史恢复与取消主要链路已实现；助手/MCP/工具/HumanGrant完整组合仍待验收。见 services/core/src/assistant_model/agent/。 |
| SUB-01 | 部分实现 | EPay一次购买的Core/Go/HTTP路径已有；其他provider首购、周期、取消及恢复逐项待核验。 |
| SUB-02 | 部分实现 | 订阅管理记录与立即失效已有；voucher、绑定删除、批量重置完整行为待核实。 |
| PAY-01 | 部分实现 | 余额/免费商店购买已通过固定组合验收；外部商店收款尚未接通。PublicConfig的platform_payment_methods/catalog为空（store/legacy_catalog.go）；external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo仍缺各自购买交付结算验收。人民币钱包充值及95笔历史充值依据已验证，属于钱包支付范围。 |
| PAY-02 | 部分实现 | 钱包Stripe/Waffo Global及受EnableLegacyRefund控制的EPay退款、商店余额全额退款已有；新订单部分退款实现见STORE-02。Pancake明确RefundEnabled=false、Refund返回ErrUnsupported（payments/pancake.go）；外部商店退款尚缺冻结交易、退款通知/查询、收入回冲及恢复全链。 |
| PAY-03 | 部分实现 | catalog/pair/save已接正常router，已有Core绑定的return URL/私钥配置可原子保存并更新运行实例；Session-only customer portal按Core用户绑定customer及单一store/environment。模块、composition、Web入口针对性测试及race通过，含并发保存、SDK本地HTTP、秘密文件持久化与配置重建。更换merchant/store/product仍需Core策略同步，当前明确返回PAYMENT_CORE_POLICY_REQUIRED；真实Core Session、PG、外部provider、浏览器及OS重启组合待验。历史缺字段订单继续在供应商I/O前拒绝。 |
| STORE-01 | 待补齐 | 游客购买与邮件提货合同未齐 |
| STORE-02 | 部分实现 | 新订单数量/金额部分退款及Core资金协议已有；历史余额退款分支仍不支持，完整组合待验。 |
| STORE-03 | 部分验收 | 当前固定组合购买、付款、库存提货、全额退款通过；显式卖家自购和三态目录的完整浏览器覆盖仍缺。 |
| STORE-04 | 待验收 | 账户隔离、购物车并发与清理有旧组件证据；最终组合的购物车、收藏、游客合并浏览器验收未完成。 |
| REVIEW-01 | 待补齐 | 市场 AI 自动审核缺配置、执行和结果历史 |
| APP-01 | 实现待验收 | 公开/私有GitHub配置、加密凭据、原子拉取及快照已有实现；实际Core与浏览器组合仍待验收。 |
| APP-02 | 实现待验收 | 实时用量来源、SVG与Core历史查询已有实现；固定进程组合、浏览器及公网来源仍待验收。 |
| APP-03 | 待补齐 | HeroSMS 当前目录、报价、购买、取消、重下单和管理缺失 |
| APP-04 | 待验收 | io.net 实现及数据库权限缺陷复验通过；真实供应商、Core 授权与浏览器完整链未验收。 |
| APP-05 | 待补齐 | 平台充值折扣码缺管理、校验与新消费合同 |
| APP-06 | 部分实现 | 原生abuse策略、标记与审计已有；公开安全报告及Moderation旧行为映射仍待补。 |
| APP-07 | 待验收 | RSS 抓取、缓存、配置有组件证据；目标 edge 路由与浏览器未验收。 |
| APP-08 | 待补齐 | 价格变更 webhook 投递与重试后台未迁移 |
| APP-09 | 待验收 | 展示汇率的权限、保存与账本不变有组件证据；公网汇率源及完整页面链未验收。 |
| CLI-01 | 部分实现 | 已有登录、模型目录、余额、预设及Pi适配；其他软件安装同步恢复仍有阻塞，多平台分发未齐，见 apps/cli/README.md。 |
| CHECK-01 | 部分验收 | 旧快照1247身份、883密钥、OAuth、8 Passkey、用户组已迁移核对；Root已正常激活，42条旧密钥及原禁用身份读回通过；旧raw/sk HTTP认证已通过；历史账号交互登录、实际模型配置及最终生产迁移待完成。 |
| CHECK-02 | 本地演练通过 | 旧快照钱包、欠额、推广余额、5托管、7计划、15历史订阅、60原订单及信任基线已导入读回；最终停写快照、生产对账与持续权益运行核验仍待完成。 |
| CHECK-03 | 待验收 | 实际历史日志、用量、统计、财务的分页、时区、unknown金额及导出逐项对照尚无完整证据。 |
| CHECK-04 | 部分验收 | Core7/Go9/Web5 正常登录、人民币模拟渠道充值、商店购买提货退款和恢复通过；历史未决付款承接、其他要求的故障场景及生产配置运行仍待核验。 |
| CHECK-05 | 部分验收 | release文字与图片、计费及UNKNOWN路径通过；助手/MCP/工具/HumanGrant完整组合与所需协议全覆盖仍未完成。 |
| CHECK-06 | 部分验收 | Web5全套检查与固定组合17项浏览器用例通过；全角色/语言/窄屏、CLI及Extore两端完整覆盖未完成。 |
| CHECK-07 | 部分验收 | Core7正常初始化、ACL、原生包校验通过，旧服务恢复已有实证；本次最终生产迁移、安装切换与恢复演练待完成。 |
| CHECK-08 | 待验收 | 已有受限资源组件运行数据；同口径新旧版性能、长稳与故障恢复完整验收未完成。 |
| CHECK-09 | 部分验收 | 固定Core7/Go9/Web5的HTTP15项、浏览器17项及release图片通过；实际历史账号、最终生产快照、目标主机和QD-001剩余条件未闭合。 |

## 当前证据入口

| 范围 | 固定证据 |
| --- | --- |
| Core7制品与准入 | `lmm-core7-release-vh29l59q/admitted-core7.receipt.private.json`，准入提交 `316ec5d`；4项历史Core65版本断言失败保留。 |
| 固定组合 | `lmm-combination-core5-web4-nt6y0hde/core7-go9-web5-combination.receipt.private.json`，SHA256 `140f2ad81f95b72d32a9f2991c6841eceff8ec31ac367123016513cf66dabb8a`。 |
| release图片 | `lmm-core7-release-vh29l59q/image-release-r2/result.json`，生成、编辑、计费及UNKNOWN路径。 |
| 身份与凭据 | `credentials-412-0e1e30sg` 中 OAuth、Passkey、routing_groups 的 exact-readback 收据；Root成功见 `trust-resumed-l2yw2und/root-owner-handoff/apply-result-normalized.private.json`及activation-readback；首次小数字面哈希拒绝保留，`8055460`生成独立规范副本，原来源不改。 |
| 资金及订阅 | `auth-continuity-rehearsal-8hqzzqo8` 中 opening、subscription、originals 的 apply/readback 收据；原OOM/UNKNOWN、只读NOT_APPLIED及恢复链保留。 |
| 商店与客服 | `full412-go-producers-84jgu6ps` 中 stock-readback、support-readback 收据。 |
| 历史充值依据 | `cc52-payment-basis-l066cykb/normal-install95-local-02/result.private.json`；写集为facts回执与95条basis，应用资金数0。 |
| 信任基线 | `trust-resumed-l2yw2und/applied-01/result.private.json` 及独立readback，1247用户、104付费投影逐字段相等；首次工具版本拒绝保留。 |

上述路径位于操作者私有缓存/状态目录，包含受限业务材料。仓库只记录证据位置，不提交秘密与用户数据。

## 范围与维护

完整要求见[产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)与[支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)。历史审计见[归档索引](archive.md)。

只更新受影响项目的当前结果、具体剩余工作和必要证据入口。源码提交、本地验收、发布与生产部署分别记录。Extore自动部署main限制继续有效。

# 新架构进度

更新于 2026-11-12。此文件维护当前状态；历史执行过程和失败记录保留在 Git 历史与对应验收收据。

## 上线现状与剩余阻塞

- **尚未上线新架构**。公开服务最近核验为旧版 `0.2.118`，HTTP 200。
- **本地迁移演练**已完成身份、资金、商店/客服、订阅、OAuth/Passkey、用户组及信任基线导入读回。管理员旧密钥最终核验仍未完成，fresh plan 当前拒绝 `SourceMismatch`，正在定位。
- **正式制品与固定组合**已验证。Core7 `e5b3e11`、Go9 `76aaf71`、Web5 `907197b` 的 HTTP15项、浏览器17项及release图片检查通过，采用隔离账号和本地供应商。Core7 Arch/DEB `0.1.19-7` 已构建及校验，当前提交与包未签名。
- **生产仍需完成**最终停写捕获、全阶段迁移与对账、部署切流和实际业务验收。旧快照 `FINAL_20261111T120751Z` 已因旧服务恢复写入而失去未来切换资格。
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
| AUTH-01 | 待补齐 | 通用 OIDC、WeChat 登录/绑定及旧 OIDC provider 合同未对齐 |
| MODEL-01 | 待补齐 | 视觉输入与 Anthropic cache_control 切片已实现；hosted tools/fileData 与组合验收仍未齐 |
| MODEL-02 | 待补齐 | Responses 持久化/previous_response_id 与部分旧同步 API 缺等价行为 |
| MODEL-03 | 待补齐 | 独立 TTS/STT/translation 和双向 Realtime/WebSocket 未齐 |
| MODEL-04 | 待补齐 | 视频、Kling/Jimeng、MJ、Suno 异步任务未形成完整闭环 |
| MODEL-05 | 待补齐 | 渠道 tag、多 key 轮换、Codex 运维和真实推理探测未齐 |
| ASSIST-01 | 待补齐 | 预设、Core/Go 实时流及 Web 原请求恢复已实现；最终组合仍待验收 |
| SUB-01 | 待补齐 | 外部支付订阅缺口：EPay 一次购买及 Stripe/Creem/Pancake 的原支持生命周期 |
| SUB-02 | 待补齐 | 管理记录与立即失效已实现；reset voucher、绑定/删除与批量重置仍未齐 |
| PAY-01 | 待补齐 | 商店仅接 balance/free，缺四类旧外部收款 |
| PAY-02 | 待补齐 | Pancake 及外部商店退款未齐 |
| PAY-03 | 待补齐 | Pancake catalog/pair/save、订阅映射与 customer portal 未接完整后台 |
| STORE-01 | 待补齐 | 游客购买与邮件提货合同未齐 |
| STORE-02 | 待补齐 | 商店按数量/金额的部分退款缺失 |
| STORE-03 | 部分验收 | 当前固定组合购买、付款、库存提货、全额退款通过；显式卖家自购和三态目录的完整浏览器覆盖仍缺。 |
| STORE-04 | 待验收 | 账户隔离、购物车并发与清理有旧组件证据；最终组合的购物车、收藏、游客合并浏览器验收未完成。 |
| REVIEW-01 | 待补齐 | 市场 AI 自动审核缺配置、执行和结果历史 |
| APP-01 | 待补齐 | 公开与私有 GitHub 配置、原子拉取及快照已实现；实际 Core/浏览器组合待验收 |
| APP-02 | 待补齐 | 用量分享 Go 实时来源、原生 SVG 与 Core 全历史查询已实现；完整验收待完成 |
| APP-03 | 待补齐 | HeroSMS 当前目录、报价、购买、取消、重下单和管理缺失 |
| APP-04 | 待验收 | io.net 实现及数据库权限缺陷复验通过；真实供应商、Core 授权与浏览器完整链未验收。 |
| APP-05 | 待补齐 | 平台充值折扣码缺管理、校验与新消费合同 |
| APP-06 | 待补齐 | 公开安全报告与 Moderation 后台未完成旧行为映射 |
| APP-07 | 待验收 | RSS 抓取、缓存、配置有组件证据；目标 edge 路由与浏览器未验收。 |
| APP-08 | 待补齐 | 价格变更 webhook 投递与重试后台未迁移 |
| APP-09 | 待验收 | 展示汇率的权限、保存与账本不变有组件证据；公网汇率源及完整页面链未验收。 |
| CLI-01 | 待补齐 | 锁定 CLI 仍为开发预览，多软件与多平台接入未齐 |
| CHECK-01 | 部分验收 | 旧快照1247身份、883密钥、OAuth、8 Passkey、用户组已迁移核对；Root fresh plan 当前 SourceMismatch，尚未激活，历史账号实际登录/密钥及最终生产迁移待完成。 |
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
| 身份与凭据 | `credentials-412-0e1e30sg` 中 OAuth、Passkey、routing_groups 的 exact-readback 收据；Root失败见 `trust-resumed-l2yw2und/root-owner-handoff/fresh-plan.stderr.private.txt`。 |
| 资金及订阅 | `auth-continuity-rehearsal-8hqzzqo8` 中 opening、subscription、originals 的 apply/readback 收据；原OOM/UNKNOWN、只读NOT_APPLIED及恢复链保留。 |
| 商店与客服 | `full412-go-producers-84jgu6ps` 中 stock-readback、support-readback 收据。 |
| 历史充值依据 | `cc52-payment-basis-l066cykb/normal-install95-local-02/result.private.json`；写集为facts回执与95条basis，应用资金数0。 |
| 信任基线 | `trust-resumed-l2yw2und/applied-01/result.private.json` 及独立readback，1247用户、104付费投影逐字段相等；首次工具版本拒绝保留。 |

上述路径位于操作者私有缓存/状态目录，包含受限业务材料。仓库只记录证据位置，不提交秘密与用户数据。

## 范围与维护

完整要求见[产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)与[支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)。历史审计见[归档索引](archive.md)。

只更新受影响项目的当前结果、具体剩余工作和必要证据入口。源码提交、本地验收、发布与生产部署分别记录。Extore自动部署main限制继续有效。

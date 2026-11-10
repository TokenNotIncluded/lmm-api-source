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

REL-01 的既有静态边界阻断已解除；REL-02 已补齐下表固定候选的真实镜像资源与启动验收。新增实现改变存储合同或运行输入时，仍须复验相应范围。CHECK-09 仍需逐条补齐 QD-001 退出证据；不得通过只改指纹、延后期限、降级债务或手填通过记录消除阻断。

2026-11-08 固定候选为 [7fb02902][rel-candidate]，基于 `bb2697e9216d67bf1f34705af570ed2d8eb5288d`；开发分别由 `rel01_boundaries`、`rel02_delivery` 负责，`rel_acceptance` 独立审查并执行检查，验证前后工作树干净。真实 PostgreSQL、容器及资金业务验收未运行。

## 工程与交付

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| REL-01 | 静态边界合同已对齐：模块连接池、4 处已审查正文指纹及 Core schema 拒绝范围 | 已验收 | 2026-11-08，`rel_acceptance` 在 [7fb02902][rel-candidate] 独立核对池归属/关闭、GRANT、namespace 与安装权限，33 项边界检查通过；补齐 directory_ads/referrals 及拒绝反例。[原始证据][rel-evidence]。范围为静态源码合同，真实数据库权限验收仍按 CHECK 项执行。 |
| REL-02 | 当前源码/main、TOML 与固定来源已对齐；固定候选真实运行镜像已验证 | 已验收 | 修复 [15bc5748][rel02-fix] 的既有静态证据保留。2026-11-10，`Maxwell` 从 `c9ed1de2deed81bc5569f0a29d18b9b57caa1ac6` 私有 git archive 实际构建 Extensions runtime linux/amd64，镜像 `sha256:47c775673f82ee8bc947d68183efd2a249a3cf9db999997af81d0a3a3c55fd21`；原版 smoke 验证资源/许可哈希、UID 65532、无网络/只读根、liveness 200 和正常退出 0，容器清理读回通过。证据：`~/.cache/lmm-rel02-c9ed1de-mhjivaya/REPORT.txt`。首次 mise 信任拦截日志保留，固定镜像只调 PATH 后复跑通过。未包含随后 APP/MODEL/ASSIST 增量；最终组合、发布和生产按 REL-03/CHECK 项继续验收，未推镜像或启用 Actions。 |
| REL-03 | 当前产品组合的制品、安装升级与发布证据未闭合 | 待前置 | CHECK-09 关闭后，关联 Core、Extensions、CLI、Web 的来源、平台、摘要/签名、安装升级记录与 Release；发布和生产切换分别记录。 |

## 身份、模型与助手

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| AUTH-01 | 通用 OIDC、WeChat 登录/绑定及旧 OIDC provider 合同未对齐 | 待补齐 | 补 provider、claims、issuer/client 校验、绑定与撤销和实际页面。保留已实现的 OAuth2 服务端、MFA 与 Passkey，分别验收。 |
| MODEL-01 | 视觉输入与 Anthropic cache_control 切片已实现；hosted tools/fileData 与组合验收仍未齐 | 待补齐 | 2026-11-09，`model_identity` 实现 [df9b3c9d][vision-input]，`root` 独立审阅：Chat/Responses 图像 URL 与内联数据、Anthropic URL/base64、Gemini inlineData；保持图文顺序/detail，拒绝有损转换，Core 不下载客户端图像 URL。`cargo check --lib --locked` 通过；此前三次 codegen 被 SIGKILL，新增单元、真实 TCP 与 PostgreSQL 计费用例均未执行，不据此声明协议/资金验收通过。[原始记录][progress-evidence]。2026-11-10，`86f6925` 保留 Anthropic 顶层、工具、system、消息块和工具结果的缓存标记/TTL/原边界，跨协议有损转换明确拒绝；`Poincare` 开发自测后，`Maxwell` 独立冻结 `86f6925`，精准 Anthropic 20 项、vision 5 项通过（25 个唯一测试）；旧宽筛选 33 次执行含重复/无关匹配，不计作 33 项覆盖。证据 `~/.cache/lmm-model01-independent-023fvl82/REPORT.md`。未验证真实提供商及计费组合，文档块/多块工具结果仍明确拒绝。继续补 hosted tools、带归属校验的 Gemini fileData/provider file_id，并运行四协议响应、usage、唯一结算与恢复测试。 |
| MODEL-02 | Responses 持久化/previous_response_id 与部分旧同步 API 缺等价行为 | 待补齐 | 补存储、续接、工具、重放与计费；逐项恢复所需 completions、compact、search、decisions、moderations 等实际行为。 |
| MODEL-03 | 独立 TTS/STT/translation 和双向 Realtime/WebSocket 未齐 | 待补齐 | 补连接、取消、回执、计费与断连恢复；当前 Chat audio 与模型 SSE 继续保留，不能据此关闭本项。 |
| MODEL-04 | 视频、Kling/Jimeng、MJ、Suno 异步任务未形成完整闭环 | 待补齐 | 按任务族补提交、查询/回调、结果/remix、原操作恢复与唯一扣费。 |
| MODEL-05 | 渠道 tag、多 key 轮换、Codex 运维和真实推理探测未齐 | 待补齐 | 补自动选择/失效/恢复与真实调用测试；现有渠道 CRUD、密钥库、价格发布和目录探测保留。 |
| ASSIST-01 | 预设、Core/Go 实时流及 Web 原请求恢复已实现；最终组合仍待验收 | 待补齐 | 2026-11-09，`model_identity` 实现 [a0804b60][assistant-presets]，`root` 在冻结输入上独立验证真实 PostgreSQL/TCP 的发布、版本冲突、匿名计数和预设点击一致性；三个 Go 定向用例、webcompat 包、实际 TanStack 父/子路由八项用例及 Extensions 构建通过。L5 可读、L6 可写；预设只提供用户开场内容，不增加工具或资金权限。[原始记录][progress-evidence]。2026-11-10，`16bd895` 接通 Go 真实逐帧 SSE、原 turn 只读重连与显式取消；`Mendel` 对 assistant/webcompat/coreclient 的定向 race 自测通过，包括真实 HTTP 首帧 flush/断开与 gRPC/TCP 协议，未使用实际 Rust Core/账本；原记录 `~/.cache/lmm-assist01-validation/report.txt`。`33ef79b` 接通 Core 原计费请求的真实上游 SSE 与绑定 owner/input/round 的 live registry；`Mendel` 在冻结私有源码上编译并自测 21 项通过，真实隔离 PostgreSQL/受控 HTTP 上游证明前缀提前可见、原 usage 唯一结算、取消/截断保留 hold 及独立实例恢复不重发。开发证据 `~/.cache/lmm-assist01-validation/core-report.md`。`Dirac` 核对固定来源与测试二进制哈希后，在新建私有 PG18.6 独立执行 21 个唯一用例（2 个 SSE、8 个 provenance、11 个 SQLx/PG）全部通过、零跳过；首次一项因验收 TMPDIR 太长失败，仅修短路径后复跑该项，失败保留。证据 `~/.cache/lmm-predeploy-validation/20261110-independent-assist-core-run/report.md`；这是独立运行开发产物，非独立编译，跨实例测试仍是同一 OS 进程内对象。实际 Watch/Cancel RPC→Go→Web、真实进程重启、真实外部提供商与最终组合未验收。`66e8e9b` 已提交 Web 首次 ready 前断线的只读定位、明确拒绝、用户显式 receipt 恢复与原 turn 取消；开发自测五个隔离 Bun 进程共 169 项通过，类型/lint/格式通过，证据 `~/.cache/lmm-api/assist-01-web-recovery/20261110T080029Z/report.md`。`0b1c3a3` 补齐 Go 同一创建键查询/receipt 恢复及持久化 pre-start fence；开发定向 race 回归与 3 项真实 PG/HTTP race 通过，覆盖只读权限、迟到 Chat 拒绝及先写入 intent 时保留 UNKNOWN，证据 `~/.cache/lmm-assist01-lookup/report.md`。后者的 Core 身份/登记端口仍为受控 fixture。随后 `Dirac` 独立运行固定 Core `7f6a…` 与 Go `ec3210a` 组件的实际进程链，通过 Watch→Go SSE、prefix 早于 done、断线原 turn GET 恢复、已知 usage 唯一扣费、取消保留待对账资金和会话/归属拒绝；[固定组件证据](../.quality/evidence/assist-osmain-20261110.md) 保留此前全部 fixture 失败。浏览器 cookie/CSRF、最终组合及不确定提供商后续对账仍未验收。Web 类型限定修复见 CHECK-06，不能代替最终组合。 |

## 订阅、支付与商店

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| SUB-01 | 外部支付订阅缺口：EPay 一次购买及 Stripe/Creem/Pancake 的原支持生命周期 | 待补齐 | 按 provider 补产品快照、首购/周期 grant、失败、取消和原交易恢复；钱包充值与余额订阅分别验收。2026-11-10，`d24577e` 提交 EPay 原生购买、冻结报价确认、原单只读 checkout 恢复和 root 价格管理页；开发逐文件自测 15 项、类型检查与隔离 Web 构建通过，新增提示七语言已补齐，证据 `~/.cache/lmm-api-sub01-web-epay/admin-v2/manifest.json`。该前端结果使用受控 HTTP/DOM fixture。`3579ac7` 已提交 Core59 EPay 原始凭据、唯一 grant、Go 持久化回调恢复及离线升级命令；开发真实 PG/TCP 四项用例通过，覆盖伪造回调不阻塞有效凭据、丢 ACK 后恢复、权限恢复后无需支付商重发、取消/改价/轮换密钥后的原交易和共享购买上限。证据 `~/.cache/lmm-sub01-validation/rust-epay-final.log`、`go-upgrade-final.log`；提交中的 810 个 Core 构建输入与 `core59-final-inputs.before.json` 一致，产物摘要 `7f6a0fdc9a5a8ca4c48fa7c05d1f639c09850af4274f0ad4b6c3d2a72d532c00`。新增管理价目 GET 尚未编译验证，未纳入该提交；实际浏览器、固定组合独立验收和其他 provider 生命周期仍未完成。 |
| SUB-02 | 管理记录与立即失效已实现；reset voucher、绑定/删除与批量重置仍未齐 | 待补齐 | 2026-11-10，`384f531` 新增 Core 管理记录、原生用户抽屉与 root 失效操作；`root` 自测后由 `Dirac` 独立在 PostgreSQL 18.6 复跑实际查询、过滤/分页、导入额度、8 次并发原操作恢复、唯一回执/审计与审计失败回滚，资金/消费/预留保全；页面四项定向自测通过。证据：`~/.cache/lmm-predeploy-validation/20261110-independent/report.md`。仅 SQL 与组件证据，Rust HTTP/会话权限/charging 进程及浏览器未验收；继续补 voucher、绑定/删除、批量重置及完整授权/额度语义。 |
| PAY-01 | 商店仅接 balance/free，缺四类旧外部收款 | 待补齐 | 恢复 external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo；分别证明卖家/平台收款、币种、库存交付、结算和恢复，保留 LDC 单位。 |
| PAY-02 | Pancake 及外部商店退款未齐 | 待补齐 | 补原交易冻结、发送、查询/通知、失败释放、对账及卖家收入回冲；已有 Stripe/Waffo Global 退款与 Core ReleaseRefund 按现合同验收。 |
| PAY-03 | Pancake catalog/pair/save、订阅映射与 customer portal 未接完整后台 | 待补齐 | 接当前配置和权限、秘密只写、server 绑定 customer/store/environment，并完成真实管理页面联动。 |
| STORE-01 | 游客购买与邮件提货合同未齐 | 待补齐 | 补游客订单归属、邮箱验证、付款领取、邮件 outbox/重试；覆盖凭证混用、重放及退款后禁止领取。登录买家的提货码/链接已有。 |
| STORE-02 | 商店按数量/金额的部分退款缺失 | 待补齐 | Core 与 Go 绑定原订单和每次退款身份；证明累计上限、库存/交付、卖家收入、外部回执及丢 ACK 恢复。 |
| STORE-03 | 三态目录、管理页与显式卖家自购已实现；实际 Core RPC 和浏览器验收待补 | 待验收 | 2026-11-09，`commerce` 实现 [63db64bc](https://github.com/TokenNotIncluded/lmm-api-source/commit/63db64bc0124f0361f0f5a7facde72b0c904d46a)、[6d74cefd][core58] 与 [9b377046][store-self-purchase]。`root` 独立复验真实 PostgreSQL 的可见性/分页、56→58 原数据与权限保全、自购全额预留/结算/退款和八组并发原操作重放；订单/提货/退款用例使用合成资金回执，实际 Core 账本 SQL 另行验证，两者不冒充 RPC 闭环。页面八项可见性测试通过，checkout 全文件 31 项通过并发现一个旧库存 fixture 缺少能力参数，在旧基线复现后补足该输入，原断言定向复验通过。能力由 `commerce.self-purchase.v1` 明确协商，`store_access_supported` 继续关闭。[原始记录][progress-evidence]。新 Core 组合编译被 SIGKILL，实际进程资金链、全量 Web 类型检查和浏览器操作仍待验收。 |
| STORE-04 | 账户购物车、收藏、清理和游客合并已接通；完整页面验收待补 | 待验收 | 2026-11-09，`commerce` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 PostgreSQL 的账户隔离、绝对数量并发重放、重启读回、私有化隐藏、暂停保留及下架/变体清理；既有库离线迁移与精确 DML 权限用例通过。已有页面按 `max(server,guest)` 合并后发送绝对数量 PUT；能力已开放，不预占资金或库存。冻结 Extensions 构建通过。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL，浏览器操作与最终组合按 CHECK-06 继续验收。 |
| REVIEW-01 | 市场 AI 自动审核缺配置、执行和结果历史 | 待补齐 | 恢复 off/assist/auto 与发布/拒绝边界，接审核设置和权限；保留现有人工审核与版本发布。 |

## 业务后台与页面

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| APP-01 | 公开与私有 GitHub 配置、原子拉取及快照已实现；实际 Core/浏览器组合待验收 | 待补齐 | 2026-11-10，`2e2ae58` 实现配置 revision CAS、受限 archive 读取、完整快照与来源持久化、operations v3 与离线升级、管理页保存版本和草稿禁止 pull；`Dirac` 独立复验 39 项边界合同、operations race 15 项零跳过、真实 TLS/PG 并发与失败保留、v1→v2→v3 原操作恢复、最小权限及 composition/离线 CLI，证据 `~/.cache/lmm-predeploy-validation/20261110-independent-app01/report.md`。Web 定向用例和完整类型检查自测通过；Core 授权为合成 fixture，当时私有仓库密钥明确 501。后续 `9c9ef8a` 补加密凭据写入/替换/清除、L6 新鲜授权、受限私仓 archive 请求、显式离线凭据扩展和独立 32 字节 keyfile。开发实际 TLS/PG/race 与 Web 定向测试通过；首次全站类型检查遇到同时编辑的 SUB 测试类型错误，后者已由 SUB 作者修复并通过类型检查。证据 `~/.cache/lmm-app01-private/report.md`。root 另完成真实 PG 离线 CLI/原收据恢复/不同操作拒绝和密钥隔离自测，证据 `~/.cache/lmm-script-cli-30zvxt6z/report.md`。私仓网络为受控 TLS fixture，无真实 GitHub PAT；独立验收、实际 Core 与浏览器全链路仍待完成。 |
| APP-02 | 用量分享 Go 实时来源与原生 SVG 已接通；Core 查询补齐及完整验收待完成 | 待补齐 | `beae53f` 接通原生 profile/models/用量卡片、按用户读取 LMM 数据和 Cursor 公开数据源，保留显式 snapshot 来源；金额未知不当作零，撤销或权限变化使在途结果失效。开发通过实际 Service.Register 路径的 9 项定向用例（含 6 个参数子场景），覆盖布局、公开字段、混合未知价格、所有者隔离、撤销及 Cursor 网络/缓存规则；11 文件运行前后哈希一致，证据 `~/.cache/lmm-app02-validation/inputs.json` 与 `test-final.log`。测试使用受控 Core/数据库/网络 fixture；Core 全历史查询和 nullable 金额/显示汇率实现仍待编译验证，真实 Core/PG、浏览器与 Cursor 公网未验收。 |
| APP-03 | HeroSMS 当前目录、报价、购买、取消、重下单和管理缺失 | 待补齐 | 补真实供应商 IO、Core 资金、退款和页面接线；现有历史只读不变。 |
| APP-04 | io.net 远端模型部署管理未迁移 | 待补齐 | 补连接设置、创建/列表、容器/日志、续期、权限与失败恢复；与本项目安装发布工具分开验收。 |
| APP-05 | 平台充值折扣码缺管理、校验与新消费合同 | 待补齐 | 补范围、用户/金额核验、次数/并发、消费绑定及旧数据迁移。商店优惠券与充值兑换码已有，属于独立行为。 |
| APP-06 | 公开安全报告与 Moderation 后台未完成旧行为映射 | 待补齐 | 对照 native abuse，逐项补 policy/stats、审核队列/任务/统计和页面操作；已有策略、标记、审计继续保留。 |
| APP-07 | RSS 抓取、缓存、设置保存与页面接线已实现；线上 edge 与浏览器验收待补 | 待验收 | 2026-11-09，`business_apps` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 HTTP 抓取、RSS/Atom/RDF 投影、并发/缓存/错误与网络边界；三项真实 PostgreSQL 用例证明配置持久化、并发版本校验、v1→v2 离线升级及原操作恢复、HTTP 权限与禁用源不抓取。七项定向 Go race 测试、冻结 Extensions 构建及 34 项边界检查通过。配置仅授予 SELECT/UPDATE，普通启动不执行 DDL。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL；实际 Nginx 路由与浏览器验收仍需目标环境。 |
| APP-08 | 价格变更 webhook 投递与重试后台未迁移 | 待补齐 | 补订阅接收人、outbox、签名 HTTPS、持久状态与幂等重试；现有站内通知另行验收。 |
| APP-09 | Core 报价读取、展示汇率保存及状态回读已补齐；实际进程与公网验收待补 | 待验收 | 2026-11-09，`business_apps` 实现 [6d74cefd][core58]，接已有 `33ba2e1` Web；`root` 独立验证真实受限 PostgreSQL 的专用 setter、输入边界、无直接表写/DDL、失败事务回滚及原始余额/账本逐行不变。现有页面七项定向用例通过，明确区分报价日期与下载时间。Core56→58 的实际 SQL 保留/权限验证通过；新组合 `cargo check --tests` 与 `--lib` 均被 SIGKILL，Rust HTTP/报价网络 fixture 未运行，Frankfurter 公网及浏览器验收未完成。[原始记录][progress-evidence]、[升级合同](commerce-display-rate-upgrade.md)。 |

## 客户端

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| CLI-01 | 锁定 CLI 仍为开发预览，多软件与多平台接入未齐 | 待补齐 | 按目标软件/版本完成真实接入、可恢复写入、凭据与失败路径、安装/同步/更新/解除/卸载/运行和签名分发；预设目录数量不代表已接入数量。 |

## 迁移与独立验收

这些项目保留已有实现及历史实测的价值；关闭时需证明当前最终组合，不从旧报告自动继承通过。CHECK-04、CHECK-05、CHECK-06 共用冻结候选和证据标识，分别证明真实宿主、协议/权限与用户操作。

| 编号 | 验收目标 | 状态 | 关闭条件与最小证据 |
| --- | --- | --- | --- |
| CHECK-01 | 身份与权限迁移 | 待验收 | 2026-11-10，`Maxwell` 冻结 `1e0937b` 核验：源码声明 58 组件，安全 importer 的 reviewed profile 仅到 56；实际调用拒绝 `component_rust_source_fingerprint_mismatch`。现存核验过的缓存 Core 分别为 54/55，不能用于当前候选。证据 `~/.cache/lmm-check0102-facts-cbpzd8wo/REPORT.md`；后续 `ccc21e9` 已审查 56→58 DDL/ACL 并补齐安全 profile 与升级收据权限检查；开发单测 167 项通过、1 跳过，真实 PG18 合成 dump→restore→身份/资金导入及两阶段重放通过，数据与序列保持一致，越权 ACL/已有升级收据实际拒绝。证据 `~/.cache/lmm-safe-importer-dkqkkslg/REPORT.md`；这是 58 profile 开发证据。随后安全 importer 已接入固定 `3579ac7` 的正式 59 profile，绑定已审源码、编译 manifest、实际 catalog、ACL 与空目标围栏；59 定向单测 62 项及历史 profile 回归 5 项通过，证据 `~/.cache/lmm-importer59-reviewed-p7mlo7oo/REPORT.md`。随后开发在初始 Core59 二进制 `07930fa8…` 上运行隔离 PG18.6 合成源 dump→restore、身份/资金 SQL 导入与即时重放，通过债务 -120、推广余额 -5、钱包 43 及高权限用户休眠回读；证据 `~/.cache/lmm-importer59-actual-run-zpr9llcm/REPORT.md`。该源仅为三用户 payload fixture，完整旧库 exporter/CLI、持续权益、原路由与凭据链及最终候选独立验收仍未完成；管理 GET 后续修改须同步精确来源指纹。未改用宽导入入口绕过。需匹配最终 Core 的实际安装与 catalog receipt 后运行真实迁移。仍须同一来源/目标的身份、特权、团队/开发者、密码/会话/Key/OAuth/MFA/Passkey 映射；实际登录/撤销、deny 优先、跨租户和并发批准，旧凭证处置与全状态回读。 |
| CHECK-02 | 资金与持续权益迁移 | 待验收 | 钱包、欠款、返佣、订阅、Key 限额、预留与持续权益具备来源和单位；逐笔/逐账户对账、账本平衡，原操作恢复不重复扣款。 |
| CHECK-03 | 历史读取与导出 | 待验收 | 实际日志/用量/统计/财务的来源目标范围、时区、分页、筛选、unknown 金额、权限和导出字段逐项对照。 |
| CHECK-04 | 支付、商店、客服及商业资金闭环 | 待验收 | 当前 Core + 实际 Go main + PostgreSQL；重复/乱序、撤权、并发消费退款、丢响应、双进程重启及配置变化后的旧订单。合成 provider 与真实支付商分别报告。 |
| CHECK-05 | 模型、助手、MCP、工具与 HumanGrant | 待验收 | 所需协议的 JSON/SSE/usage/取消/重试/冻结价格；真实对话、赞助付款方、跨租户/撤权/额度反例、恶意工具输出与原操作恢复。 |
| CHECK-06 | 全站页面、CLI 与跨仓库集成 | 待验收 | 2026-11-10，`b2e97f9` 修复 Extore 目录数组交叉类型与助手测试事件类型；`Dirac` 独立执行完整 Web 类型检查及逐文件组件测试（Extore 1 项、助手设置 8 项）通过，固定输入与限制见 `~/.cache/lmm-predeploy-validation/20261110-independent/report.md`。仍需按角色/语言/桌面与窄屏/错误状态执行，截图、请求与数据库回读绑定最终候选；实际管理弹窗、OAuth consent、广告与 Extore 两端授权/多规格/重复同步/撤销/隔离。Extore main 自动部署限制继续生效。 |
| CHECK-07 | 安装、升级、备份恢复与切换 | 待验收 | 当前 schema 的 owner/runtime 分离、普通启动无 DDL、低权限拒绝；实际隔离恢复、writer fence/drain、死 holder CAS、有效写入保全与业务重启。 |
| CHECK-08 | 资源、旧新版性能与稳定性 | 待验收 | 相同 CPU/内存、数据、功能和计费语义下的优化构建；完整成功率、吞吐、P95/P99、RSS/PSS、长稳、慢消费者和故障恢复；产品 cgroup 与整机预算分开。门槛见[质量指南](quality.md#独立业务与资源验收)。 |
| CHECK-09 | 最终冻结组合与 QD-001 关闭 | 待前置 | 前置实现、迁移和验收全部关联同一源码/产物/配置/来源/目标；不同人员/agent 独立复验，逐条满足 QD-001 的真实 host/funds 条件，保留失败记录并确认无输入漂移。2026-11-10，`Peirce` 在固定初始 Core59 二进制与 `ec3210a` Go 主机产物上独立执行同一订单双 HTTP settle；两请求实际锁等待重叠，均返回相同成功回执，唯一 capture/settle operation 与余额、账本平衡回读通过。[组件证据](../.quality/evidence/qd001-same-order-settlement-20261110.md) 保留前两次 fixture/清理失败及来源漂移范围；初始组件不含后续 SUB/APP 改动，最终冻结组合仍须复验，QD-001 保持 open/critical。 |

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
[vision-input]: https://github.com/TokenNotIncluded/lmm-api-source/commit/df9b3c9d7c767248f9060905e24dd5c4212b065e
[core58]: https://github.com/TokenNotIncluded/lmm-api-source/commit/6d74cefd830e2805cfc4ac42e9558ca95a8cde9f
[assistant-presets]: https://github.com/TokenNotIncluded/lmm-api-source/commit/a0804b601375914109ca1598f5d26e82c38af38e
[store-self-purchase]: https://github.com/TokenNotIncluded/lmm-api-source/commit/9b377046b82b89367c21474505e8bfb577a1bdb8

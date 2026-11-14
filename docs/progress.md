# 新架构进度

更新于 2026-11-15。此文件只维护当前状态、剩余阻塞和证据入口；执行过程、逐轮测试及历史失败保留在 Git 历史与对应验收收据。

## 当前状态

- **共享界面与钱包留白整理（固定 `249ed44`，未部署）**：共享卡片、表单、表格、控制台与公开页面基础样式减少装饰边框和重阴影，扩大页面及区块间距；钱包突出余额、报价与主要操作，保留同期开发者工作区更新。[固定源码](https://github.com/TokenNotIncluded/lmm-api-source/commit/249ed44e9337ad35160f2a3703bf434f19d50c43) 的 Web 生产构建与首屏检查通过，压缩首屏资源 594,623 字节（上限 786,432）；最终登录与数据表布局定向自测 6 项通过，前一合并阶段登录、价格与开发者页 23 项通过。原始导航文案断言失败在变更前基线也能复现；钱包复测 8/8 通过。先前定向 lint 的两项既有 effect 问题、两次类型检查退出 137 及浏览器视觉检查受阻均如实保留；已做开发自测，独立验收未运行。静态构建、版本说明和原始日志已保存于交付包 `lmm-api-visual-refresh-249ed44.zip`，不代表生产或全站视觉验收通过。

- **前端视觉整理（源码已提交，视觉验收待完成）**：首页移除装饰性编号、坐标、光晕和重复宣传条，连接说明采用开放排版；登录注册保留原生表单与可选游戏，取消旋转环、玻璃内框和重复标题；定价页收敛标题与卡片层级，保留金额、单位、缓存价格和筛选。已合入共享界面留白改动，保留并行开发者页与钱包工作。实现提交的 22 项定向自测及改动范围 lint 通过，静态预览与生产构建通过，首屏压缩资源 595,203 字节（上限 786,432 字节）；全站类型检查两次被系统终止（退出 137），没有通过结果。Chromium 启动被环境拒绝，共享浏览器连接及桌面交互失败，截图与独立视觉验收未完成；没有部署。实现见 [`b55324f`](https://github.com/TokenNotIncluded/lmm-api-source/commit/b55324f3319b7ac1bded4faa7bb9ffff816b8a47)，界面规则见 [Web Design](../apps/web/DESIGN.md)。

- **支付网关补齐（本地候选，未部署）**：修复 Creem 展开式商品引用、EPay/Waffo 查询交易绑定及 Linux DO/CNY 主机隔离；EPay/Stripe/Creem/Pancake 新订阅购买开关与钱包分离，旧单读取和回调保留。新增 Creem 周期订阅、独立 Core/Go schema 与离线升级，接入 Stripe/Creem 购买恢复、取消续订及价格管理 UI、七语言。Pancake 外部退款回调绑定原交易与实际金额，自动退款申请继续关闭。退款持久查询任务保留原操作，不重复 POST；独立 PG/OS 进程复验发现 unsigned 成功观察会抑制 signed 失败，已修复观察隔离与旧 done 事件重投恢复。v2 固定产物通过真实进程重启及原失败库克隆恢复，原 key/hold/proof 不变且只释放一次；Core 为显式替身，不抵扣真实资金账本，原失败与复验均保留于 `/tmp/lmm-payment-independent-20261115/REFUND_LOOKUP_V2_ACCEPTANCE.md`。最新 Go 支付、RPC 客户端、装配、Web 接口、应用及安装工具 race 自测通过；43 项无 fixture 跳过不计通过。Creem Go/升级 CLI 隔离 PG 与两套 UI 定向自测通过；中央构建退出码未知且仅存旧可执行文件，本轮 Core UNKNOWN／未签发，真实账本用例未运行；后续须从不可变归档重新构建。商店四种外部支付方式仅有未注册 Core 草稿，目录、RPC、checkout、退款执行与收入回冲尚未接通，钱包结果不能抵扣。真实商户、全链路浏览器、生产启用与部署未验证，不能据此宣称全部网关已可用。

- **生产 Native69 Root1 权限恢复**：按冻结源 Root100 的实际权限及四条显式 allow（无 deny），使用目标 `lmm_core_native_20261115` 的 NOLOGIN owner 事务补齐缺失的 `core.permissions.v1` header、九行权限和 canonical payload；直接编译 sealed69 编码函数确认 242 字节及哈希一致，并写入一次审计。生产数据库读回通过，现有用户 `/api/data`、`/api/data/flow`、`/api/data/users` 已由 403 恢复 200；独立验证 owner 随后使用原 Root1 会话确认用户目录显示 1249 位用户，Nginx 回读 `/core/v1/admin/users`（20:03:56Z）及 `/core/v1/admin/system-resources`（20:04:27Z）均为 200，管理统计/日志接口也为 200。Root1 代际与 MFA、导入事实保留；6/67 仍停用且无策略。未创建 Session、未读取 Cookie/token、未部署新 Core。证据：`fresh69-forward-preparation-26752cdaae25/plans/root1-permission-recovery-20261115/RESULT.json`。

- **旧架构运行兼容清理**：Web 的 OAuth state/绑定回退、嵌套状态包裹、旧 URL 别名、双架构钱包/订阅/促销页面和无规格商品购买分支已统一为原生合同；签到只接受原生精确金额与服务端日历，领取携带操作键，待办分页与未读计数由服务端提供。模型元数据使用原生路由管理，保留由 Extensions 支持的 io.net 部署。Core/Extensions 移除环境兼容启动模式，现行调用方改为显式 TOML；历史导入执行器、冻结结构、旧订单及资金权益合同保留。相关 Web/Go 定向开发自测及 Web 构建、bundle 检查通过；性能汇总仅使用原生计数器，不再平均旧响应，用户列表及待办跳转统一原生查询字段。全局 Web 类型检查仍有未改动的 `prompt-input.tsx:103` 未使用变量错误，部分既有 effect lint 错误保留；Core CLI 解析器验证通过，定向编译仍在等待共享 target，完整运行、数据库及独立业务验收未执行。生产、提交推送与父 gitlink 均未变更。

- **找回密码入口**：登录页将“忘记密码”设为独立链接，密码登录关闭时仍可访问；邮件找回页新增返回登录。登录组件 47 项自测、范围 lint 与 390px 本地浏览器往返跳转通过；浏览器使用合成接口，未验证真实邮件送达。当前全站类型检查仍受 `prompt-input.tsx` 未使用函数及并行 `route-aliases.test.ts` 类型错误阻断；未执行独立验收或部署。

- **计费通用运算复用**：普通 Token、按次、工具、grounding、上游成本、阶梯、图片及音频计费复用现有 `num-rational` 的精确舍入；图片按张计费以有理数库替代手写 GCD 和交叉约分；旧阶梯日历使用现有 `chrono`，保留固定 UTC+8、星期编号及原时间范围。无新增依赖，最低扣费、折扣顺序和溢出边界保留。新增半值、零费用、整数边界与闰日/上海午夜用例；共享 Core 测试二进制定向自测 105 项通过、0 失败，旧 Go VM 对照用例因缺少私有 `LMM_LEGACY_TIERED_ORACLE` fixture 按原设置跳过。格式、差异和文档检查通过；数据库并发与支付业务独立验收未执行。

- **登录注册专项开发自测**：统一注册与改密的 Unicode 密码长度边界；Passkey-only 保留用户名并阻止密码提交；登录、注册、找回密码在 Turnstile 过期及提交后清除旧令牌，邮件验证码发送失败也重建控件；原生联合认证不再把已替代的旧微信/OIDC 计为可用入口。定向组件回归通过；本地真实浏览器使用合成 API/Turnstile 验证三表单过期拦截、失败重试及窄屏 Passkey 入口（`/tmp/lmm-auth-browser-check.log`、`/tmp/lmm-auth-passkey-mobile.png`）。本轮构建及包体检查通过（`/tmp/lmm-auth-final-build.log`）；随后并行路由删除导致开发服务器热更新报缺失 `api-keys`/`settings`，不抵扣该后续工作区状态。真实第三方授权、邮件送达、实体密钥及独立验收未执行；全站类型检查被既有 `prompt-input.tsx` 未使用函数阻断，改密弹窗既有 effect lint 错误保留。

- **本任务尚未执行生产切换**。2026-11-14 公开状态接口只读返回 HTTP 200、`success=true`、版本 `0.2.118`，仅证明该接口可达；当前所有迁移材料均为 rehearsal-only，本任务未对生产停写、写入或切流。
- **原生候选与当前源码分别记录**。已有独立封印的 69 组件原生产物及资源验收；当前源码包含尚未部署的 70 组件托管应用，生产 lib/bin 编译检查通过。此前 70 候选的严格 Clippy、27 项 PostgreSQL 与真实 600 秒恢复结果只覆盖其原固定输入，不自动覆盖新密钥隔离补丁。当前 Web 类型检查、生产构建及包体检查通过，定向认证、绑定、日期和 API Key 回归通过；全量 lint 仍失败，新组合全套未重跑。旧失败和 UNKNOWN 收据保留。Magpie 独立客户端与预算已纳入合同，原生插件及端到端接线仍未完成。
- **已授权立即部署并只向前迭代**。先固定可运行的来源与产物，执行最终停写、迁移核账和切流；上线后修复并继续更新，不安排数据库或版本回滚。历史文件移出服务器，未完成的功能与验收如实保留。
- **独立应用授权正在补接**。Core/CLI 共享的安装绑定、实时状态、有限模型预算和六百秒轮换恢复合同已独立审阅并推送；工作区第 70 组件及精确 69→70 升级正在实现、尚未提交；定向测试目标编译，以及授权、预算和 fresh70 数据库用例已通过；69→70 升级仍有两项失败，完整轮换恢复合同也未验完。此前 69 组件候选和测试只覆盖各自冻结范围。
- **历史 66 组件部署演练已跑通主要链路**。新装、`check-db`、九库备份与恢复、Core/扩展宿主启动及独立重启已有历史自测。当前 69 组件离线准入已独立终审，本地审核文件和双摘要已原样封印；身份迁移套件已通过（显式 PostgreSQL 夹具用例未运行），源码及审核文件漂移仍被拒绝；资金导入在此前 Native69 封印下完整 45 项自测通过，实际退出码为 0、输入无漂移；派发重试补修改变了 Core 源码摘要，新候选迁移准入需重新独立审核，旧封印不自动沿用；同库 Native69 身份导入、重放及负例已实跑；固定旧源码的新隔离 source 演练已完成；补充演练使用可验证 VCS 构建，完整 producer 前后 ACL、函数、序列及经济数据对照已捕获，进程真实退出码为 0，独立来源保留性验收已通过；完整金融审核仍未签署，历史 JSON 单位策略兼容补修已独立审核、自测并推送，新来源 archive/export 的离线身份 provenance、官方财务 plan/manifest 和完整单位事实工具已生成并实际退出 0，输入输出摘要及无批准标记已独立复核；仅历史 Native69 技术范围，新来源目标身份导入与金融批准仍未完成；空队列演练不证明非空 flush 或完整在途权益。尚未财务导入，不替代缺失的历史排空证据。正式迁移与最终停写演练仍未完成。
- **历史演练的身份与资金数据已导入并核对**。该快照目标包含 1247 个身份、4990 个账本账户、883 个 API Key、21199 条路由组、193423 条历史及 500 条 OAuth；保留 5 个托管项目、45 笔未决订阅责任及原钱包负债。真实 raw/sk Key 已完成本机 HTTP 鉴权，未外呼供应商。
- **八个历史扩展目标库已恢复**。store、support、payments、operations、community、acquisition 曾逐表比对；assistant、toolmarket 曾导入回读。商店与客服的用户引用错配已修复并推送，旧演练目标不能据此自动修复，需在全新目标重导验收。
- **完整模型/RPC 恢复配置已启动并 ready**。883 条 Key 映射保持不变；审核式重绑定解决了别名与原数据库名/OID/nonce 的绑定问题。
- **上线阻塞仍在**：最终停写捕获、全阶段迁移与对账、部署切流、全站业务验收、负载/容量验收及独立验收未完成。旧快照 `FINAL_20261111T120751Z` 已失去切换资格。
- **旧商店与工具市场内容允许重建，不再阻塞上线**。密钥与 50 条库存、13 条工具凭据的只读验证证据保留，不再补旧内容恢复验收。已有订单及资金责任仍需核对；15 份历史非活跃订阅仍待最终候选复核。
- **历史支付义务仍未结清**：95 笔字段完整的充值依据已在本地读回；81 笔缺历史字段、13 笔 ePay 缺冻结付款快照及 3 笔孤儿记录继续保留未决权益与补证路径。60 条历史订阅订单保持 UNKNOWN，不视为已自动结算。充值合同见[旧充值义务](legacy-payment-obligations.md)。

- **Pancake 充值回调补修（本地）**：原生 `top_` 订单从持久化订单定位渠道，并用下单时冻结的店铺/商品解析回调；旧 webhook 地址不再按当前店铺误选原生订单渠道。停用渠道仍可处理旧单，跨渠道/币种/环境、畸形原生引用及重复 JSON 字段拒绝；原始回调字节继续交 Core 验签，HTTP ACK 不代表到账。新增定向路由与重复通知自测通过；使用内存存储和受控 Core，不代表真实商户、PostgreSQL 持久性或独立验收。原 Core 支付策略及旧渠道验证材料仍须保留，自动退款与订阅未在本次补修范围内。

## 首发范围

迁移工具本地补修：Passkey 缺少目标用户、Telegram 标识非法或重复时，历史 cutover planner 现在拒绝整批计划，不再跳过后继续导入。扩展 `report.json` 新增逐模块 `coverage`，分别列出非空归档、缺后端、待 Core 接管与丢弃计数；`reconciled` 仍只表示目标行数一致，`runtime_coverage_verified=false`，不能作为完整迁移结论。两套定向 Python 自测共 24 项通过；真实 PostgreSQL 导入及独立验收未执行。旧 MFA 恢复码、特殊 Key 重签、其他 Passkey 公钥格式及历史订单兼容缺口仍未解决；本次工具源码变化也不继承旧迁移封印。

本次迁移允许重新创建旧商店商品、库存及工具市场内容，不以这些旧内容及其凭据的完整恢复作为上线门槛；这不取消商店和工具市场功能要求，也不豁免已有订单、付款、退款、账本及持续权益的核对与恢复责任。

首发必须支持现有账号与密钥、文字和图片模型 API、充值计费（含人民币）、商店购买/库存/提货/退款、订阅及既有数据权益。未使用功能和非关键问题允许延期；Gemini 音频已延期，zcall 更新下版本处理。以下项目保留完整产品范围，未使用功能不自动成为首发阻塞。

## 验收状态

| 编号 | 当前状态 | 当前结论与剩余工作 |
| --- | --- | --- |
| REL-01 | 历史范围已验收 | 静态边界合同已验收；当前运行权限见 CHECK-07。 |
| REL-02 | 历史范围已验收 | 旧候选镜像资源与启动已验收；当前原生制品见 REL-03。 |
| REL-03 | 部分完成 | Core、Extensions、CLI 原生候选已构建并通过 ABI 检查；Core 69 组件原生初始化与受限 runtime 复验通过。剩最终 Web 组合、资源及迁移资格、签名/发布、生产安装与运行核验，CLI 分发见 CLI-01。 |
| AUTH-01 | 部分验收 | 正常 OAuth 回调、claims 拒绝、同会话重启与 refresh 轮换已有真实 TLS 浏览器证据；WeChat 绑定解绑、绑定锁竞争及用量读取权限已有独立证据。旧回调不覆盖所选账号，以及在途取消原/新家族撤销均已通过独立浏览器与 SQL 核验。独立负例发现仅删除取消标记可复活旧流程，以及无 Web Locks 时迟到响应可清除所选账号 Cookie；已补修并通过定向自测与新冻结限定安全复验；冷双标签页正常及取消仍通过，首次夹具失败保留。窄屏提示遮挡已补修并通过新构建七语言独立视觉复验。Cookie 恢复提示边界、过期旧凭证且刷新 Cookie 已丢失、旧 provider 配置迁移与正式配置仍待验。 |
| MODEL-01 | 部分实现 | 文字、视觉输入、Anthropic 缓存及 Responses 结构化输出转换已有独立验证；hosted tools/fileData 完整入口与转换覆盖待核实。 |
| MODEL-02 | 部分实现 | 原生 compact 与 JSON/SSE 加密重放已通过独立验收；exact 不支持路径在扣款前拒绝，JSON/SSE 检查点绑定实际发送字节。HTTP store/previous_response_id、授权与普通控制参数、tiered 计价（含非零 12 小时缓存）已通过固定制品独立 PG 验收；WebSocket 生产接线与旧行为等价仍待核实。本地 GET/DELETE 资源接口并非旧版要求。 |
| MODEL-03 | 部分实现 | Chat 音频多轮历史、计费与恢复已通过独立 PG 验证；独立 TTS/STT/translation 及双向 Realtime 未齐。Gemini 音频按决定延期。 |
| MODEL-04 | 待核实 | 视频、Kling/Jimeng、MJ、Suno 完整异步闭环尚无充分证据。 |
| MODEL-05 | 部分实现 | 渠道目录、余额探测、原生 tag 与迁移规划已有实现和本地证据；8 个在用模型已书面延期。演练中的文字、Responses、图片请求已完成，相关 reconcile hold 已按授权零结算释放；生产发布与完整业务验收未完成。 |
| ASSIST-01 | 实现待验收 | Core/Go 实时流、Web 历史恢复与取消主要链路已实现；助手/MCP/工具/HumanGrant 完整组合待验。 |
| SUB-01 | 部分实现 | EPay 一次购买路径已有；其他 provider 首购、周期、取消及恢复待逐项核验。 |
| SUB-02 | 部分实现 | 订阅管理记录与立即失效已有；voucher、绑定删除、批量重置完整行为待核实。 |
| PAY-01 | 部分实现 | 余额/免费商店购买与人民币钱包充值已有本地证据；外部商店收款及各外部支付方式的购买、交付、结算待验。 |
| PAY-02 | 部分实现 | 钱包退款与商店余额全额退款已有；外部商店退款缺冻结交易、通知/查询、收入回冲及恢复全链。 |
| PAY-03 | 部分实现 | catalog/pair/save 与 Core 绑定配置已有；更换 merchant/store/product 仍需 Core 策略同步，真实 Core Session、PG、外部 provider、浏览器及 OS 重启组合待验。 |
| STORE-01 | 部分实现 | 匿名提货、非零价购买→邮件→提货及 PG/产品重启已独立验证；丢 SMTP 确认保留 UNKNOWN 不重发，旧订单保持原登录边界。游客购买待补。 |
| STORE-02 | 实现待验收 | 新订单部分退款、历史余额退款授权导入及 Core/Go 协议已有；导入器已修复用户引用与账本账户 ID 混淆。当前程序集迁移准入、历史退款唯一返还及恢复的真实组合仍待验。 |
| STORE-03 | 部分验收 | 当前 Core/Go 的购买、提货、全额退款及重放已通过真实进程验收；足额自购、不足额拒绝、同钱包退款不增币、三种可见性和改私有后既有订单权益均已独立验证。完整浏览器与最终发布制品组合仍待验。 |
| STORE-04 | 待验收 | 账户隔离、购物车并发与清理有组件证据；最终组合的购物车、收藏、游客合并浏览器验收未完成。 |
| REVIEW-01 | 部分验收 | Go 审核与恢复链路已有真实 PG 证据；Core 分类/签名批准、最终跨语言组合和浏览器待验。 |
| APP-01 | 实现待验收 | GitHub 配置、加密凭据、原子拉取及快照已有实现；实际 Core 与浏览器组合待验。 |
| APP-02 | 实现待验收 | 实时用量来源、SVG 与 Core 历史查询已有实现；固定进程组合、浏览器及公网来源待验。 |
| APP-03 | 部分实现 | Go 目录、报价、购买/取消、投诉及管理接口已接线；Core 直接供应商调用、报价与资金状态待真实 PG 验收，正式供应商、进程组合与浏览器未验。 |
| APP-04 | 待验收 | io.net 实现及数据库权限缺陷复验通过；真实供应商、Core 授权与浏览器完整链未验收。 |
| APP-05 | 部分实现 | Core 管理、冻结报价、占用与入账消费及 66→67→68 分段升级已有 PG 验证；管理页、批量生成丢响应恢复、删除保留旧占用已通过独立浏览器/SQL 验收。provider 执行与历史优惠连续性待补。 |
| APP-06 | 部分实现 | 原生 abuse 策略、标记与审计已有；公开安全报告及 Moderation 旧行为映射待补。 |
| APP-07 | 待验收 | RSS 抓取、缓存、配置有组件证据；目标 edge 路由与浏览器未验收。 |
| APP-08 | 实现待验收 | 加密偏好、签名投递、持久重试和前端设置已有；旧用户偏好续接、旧待投队列及真实 PG/TLS/固定组合待验。 |
| APP-09 | 待验收 | 展示汇率的权限、保存与账本不变有组件证据；公网汇率源及完整页面链未验收。 |
| CLI-01 | 部分实现 | 只读登录、目录、余额、预设及 Pi 适配已有实现；客户端 managed lifecycle/CC Switch 已实现，Linux 官方安装与回执恢复已实跑。Core 安装绑定授权及轮换恢复已在工作区补接并通过定向 PG 用例；第 70 组件升级与完整恢复合同验收尚未完成；真实授权同步、会话恢复与多平台分发待验，旧 69 组件证据不自动覆盖新组件。 |
| CHECK-01 | 部分验收 | 旧身份、密钥、OAuth、Passkey、用户组及 raw/sk HTTP 鉴权已迁移核对；历史账号交互登录、实际模型配置及最终生产迁移待完成。 |
| CHECK-02 | 历史演练通过 | 旧钱包、欠额、推广余额、托管、订阅、订单及信任基线曾导入读回；当前 69 组件迁移准入、最终停写快照、生产对账与持续权益运行核验仍未完成。 |
| CHECK-03 | 部分验收 | API 密钥只读 HTTP 查询已核对；管理员 Session、统计/财务页面、时区展示和 CSV 导出逐项对照仍未完成。 |
| CHECK-04 | 部分验收 | 固定组合登录、人民币模拟充值、商店购买提货退款和恢复通过；历史未决付款、其他故障场景及生产配置运行待验。 |
| CHECK-05 | 部分验收 | release 文字与图片、计费及 UNKNOWN 路径通过；助手/MCP/工具/HumanGrant 完整组合与协议全覆盖未完成。 |
| CHECK-06 | 部分验收 | 固定组合浏览器用例通过；全角色/语言/窄屏、CLI 及 Extore 两端完整覆盖未完成。 |
| CHECK-07 | 部分验收 | 初始化、ACL、原生包校验及旧服务恢复已有证据；最终生产迁移、安装切换与恢复演练待完成。 |
| CHECK-08 | 待验收 | 同口径新旧版性能、长稳与故障恢复完整验收未完成。 |
| CHECK-09 | 部分验收 | 固定组合 HTTP、浏览器及 release 图片通过；QD-001 六项资金退出条件已独立复核关闭；实际历史账号、最终生产快照及目标主机验收仍未闭合。 |

## 证据入口

| 范围 | 证据位置 |
| --- | --- |
| Core7 制品与准入 | `lmm-core7-release-vh29l59q/admitted-core7.receipt.private.json` |
| 固定组合 | `lmm-combination-core5-web4-nt6y0hde/core7-go9-web5-combination.receipt.private.json` |
| release 图片 | `lmm-core7-release-vh29l59q/image-release-r2/result.json` |
| 当前 Web 构建检查 | `lmm-completion-20261113/web-build-only-candidate-1794657376/qa-receipt.json`；冻结 2,824 个输入、五项构建检查通过且无漂移；明确未运行完整测试，不抵扣 lint、业务或生产验收 |
| 当前财务导入完整自测 | `lmm-completion-20261113/financial-exit-verified-1794653222/receipt.json`；45 项、实际退出码 0、四个输入摘要不变，不替代真实旧快照导入与独立资金验收 |
| Native69 派发补修资源范围 | `lmm-completion-20261113/postfix-resource-preparefix-1794661473/REPORT.md`；实际原门槛通过，仅固定 69 组件候选；旧 boot 中断退出 UNKNOWN 保留，不是新 70 资格 |
| 补充来源与单位事实 | `lmm-completion-20261113/source-resume03-independent-1794659908/REPORT.md`；`~/.local/state/lmm-api/supplement03-native69-offline-tech-1794663446/attempt02/REPORT.md`；完整单位事实仅技术范围，独立核对见 `lmm-completion-20261113/source-fullfacts-independent-20261114/result.json`，无经济批准或目标写入 |
| 工作区 70 组件定向状态 | `lmm-completion-20261113/managed70-hooke-20261114/attempt06/compile-result.json` 与 `pg-attempt03/RESULT.json`；定向编译通过，授权/预算/fresh70 通过，升级两项失败及未运行合同保持未完成 |
| 身份与凭据 | `credentials-412-0e1e30sg`；Root 激活见 `trust-resumed-l2yw2und/root-owner-handoff/` |
| 资金及订阅 | `auth-continuity-rehearsal-8hqzzqo8` |
| 商店与客服 | `full412-go-producers-84jgu6ps` |
| 历史充值依据 | `cc52-payment-basis-l066cykb/normal-install95-local-02/result.private.json` |
| 历史查询导入 | `history412-readonly-j6_q5udc/applied-01/readback.private.json`；只读 API 见同目录 `http-01/result.private.json` |
| 信任基线 | `trust-resumed-l2yw2und/applied-01/result.private.json` |
| 市场审核 Go 恢复 | `pg-market-review-independent-ids9wytd/receipt.json` |
| HeroSMS HTTP 接线 | `herosms-go-tcp-independent.log` |
| 扩展注册与路由 | `go-plugin-targeted.jsonl`、`go-webcompat-final.jsonl`、`go-build.log` |
| Responses 旧行为范围 | `lmm-completion-20261113/model02-legacy-behavior-review.md`；对照固定 `go-v0.2.118`，源码核对，不是业务验收 |
| HTTP 连续性与缓存计费 | `lmm-completion-20261113/continuity-independent-b10-ed2d891190/HANDOFF.md`；固定 b10 的完整上下文、非零 12 小时缓存、重启/恢复已独立验证；`continuity-b10-wire-ack-1794596669/RESULTS.md` 仅覆盖 scalar/JSON 的三阶段真实 COMMIT 丢确认，不涵盖全部模式或真实付费供应商 |
| 原生 compact 开发自测 | `lmm-completion-20261113/compact-freeze-r3/report.json`；绑定最终格式候选；此前开发 PG 自测 9 项与共享输入回归 5 项通过 |
| compact 独立业务验收 | `lmm-completion-20261113/compact-independent-r3-e0f8c4ed/report.json`；最终制品独立业务 4 项、共享输入 5 项通过；保留原始两项失败与修复复验，不覆盖真实供应商或生产验收 |
| Core 模型与原生折扣码 | `lmm-completion-20261113/independent-core/report.json`；涵盖音频、Responses 转换、折扣码与 66→67 升级 |
| Web 折扣码报价与确认入队 | `lmm-completion-20261113/discount-independent-e924a11361/report.json`；真实 HTTP→Go→Core RPC→PG，不含 provider 执行 |
| Core 折扣码管理与 67→68 升级 | `lmm-completion-20261113/core-discount-management-68-report.md`、`core-discount-name-search-report.md`；四角色升级、数据/ACL 保留与名称搜索回归 |
| 折扣码管理页与批量恢复 | `lmm-completion-20261113/admin-browser/run-97fa78b790e7/result.json`；Core/PG/浏览器/SQL，含七语言窄屏与名称搜索；不含钱包 provider 执行 |
| 匿名提货 | `lmm-completion-20261113/independent-store/final/report.json`、`postgres-report.json` |
| 商店目录与结账回归 | `lmm-completion-20261113/go-store-independent-1096761-e5253ab52e/HANDOFF.md`；绑定 `1096761`，原失败组及完整 store PG 独立复验通过；合成资金边界，不抵扣真实 Core 钱包组合验收 |
| 旧密钥与库存密文回读 | `lmm-completion-20261113/server-crypto-recovery/recovery-receipt.json`；两服务器源端 50/13 解密及现有目标库存 50 逐条内容一致；无生产写入，不代表最终停写迁移或库存业务验收 |
| 提货邮件开发自测 | `lmm-completion-20261113/store-mail-delivery/delivery-report.json`；TLS/PG/worker/race 与 SMTP 认证，不抵扣真实付费组合独立验收 |
| 付费邮件独立验收 | `lmm-completion-20261113/independent-paid-mail/INDEPENDENT-PAID-MAIL-REPORT.md`；精确 Core 基线 + mail-only Go，含实际 PG/产品重启、TLS 与丢确认后实等；不含外部邮箱/真实供应商 |
| 支付签名过期与重启 | `lmm-completion-20261113/payment-expiry-79d9d4af2e73/report.json`；真实等待 302 秒，Go 进程重启、Core 服务对象重建 |

上述路径位于操作者私有缓存/状态目录，包含受限业务材料。仓库只记录证据位置，不提交秘密与用户数据。各收据仅证明其绑定版本与范围，不代表当前源码或整站已验收。

## 范围与维护

完整要求见[产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)与[支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)。历史审计见[归档索引](archive.md)。

只更新受影响项目的当前结果、具体剩余工作和必要证据入口，不追加逐轮执行日志。源码提交、本地验收、发布与生产部署分别记录。Extore 自动部署 main 限制继续有效。

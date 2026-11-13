# 新架构进度

更新于 2026-11-12。此文件维护当前状态；历史执行过程和失败记录保留在 Git 历史与对应验收收据。

## 上线现状与剩余阻塞

- **2026-11-13 本机部署演练正在收口，尚未发布或切换生产**。已修复新装 `check-db` 仍要求已移除签到 schema、工具市场错误版本返回 `not_found`、正式 Docker 构建漏收授权 SQL、DEB 仅接受 `libssl3t64` 而排除 Debian 12 `libssl3`，以及 Web 导航接口接入后的测试夹具；新增并审核 66 组件 `fresh66-module-split` 迁移 profile，保留旧 profile 封印和移除组件的锁表差异。真实 66 组件新装、`check-db`、Go 全量测试、Web 738 个源码加 7 个脚本测试、类型检查通过；29 项相关 profile 定向测试通过。历史基线夹具已改为读取固定 Git 历史树并补齐权限输入，保留漂移拒绝断言；完整 133 项通过（1 项按原条件 skipped）。ArchDmit 当前 `lmm_api` 已只读导出并在私有 PG18.6 恢复，214 表行数对照时 212 表相同，另两表因线上继续写入而增加；八个生产扩展库也已实际恢复。当前发布目标已提交 1247 人身份、4990 个账本账户、883 个 API Key、21199 条路由组、193423 条历史及 500 条 OAuth；保留 5 个托管项目、45 笔未决订阅责任及原钱包负债。真实 raw/sk Key 已分别完成本机 HTTP 鉴权（授权范围不足时为 403，无效 Key 为 401），未外呼供应商。发布库的完整备份已独立恢复，权限收紧后 `check-db` 和实际启动通过。八个独立扩展目标库已建立最小权限身份；store/support 及 payments/operations/community/acquisition 的现有快照已恢复并逐表比对内容摘要，assistant 已导入 1006 会话、3566 历史消息、377 回执，toolmarket 已导入 7 服务、13 版本、42 工具并解密回读新目标快照。修复只读 Core 身份回执默认 owner 绑定被误拒的问题，扩展迁移器 15 项测试通过。本机 Core 与六模块扩展宿主已实际启动，公开商店读取、模块鉴权、401/404 拒绝及扩展独立重启通过；重启后选定有效真实 Key 的 raw/sk 均通过鉴权并因模型组权限得到 403，无效 Key 为 401。资金迁移套件 109 项通过。最终九库备份已生成并验证目录，最新 Core 恢复库 `check-db` 通过；完整模型配置最初因 Key 别名绑定原数据库名/OID/nonce 而拒绝启动，随后新增审核式重绑定工具，真实 883 条映射保持不变，重绑定后的完整模型/RPC 恢复配置已实际启动并 ready。50 条商店库存密文已原样保留，但缺现有部署库存密钥，未验证解密或库存操作；13 条旧工具市场凭据因原密钥缺失未迁移。15 份历史非活跃订阅仍受源时区证据合同阻断；全站业务、负载/容量和独立验收仍未完成。所有材料均标记 rehearsal-only，生产未停止、未写入、未切流。

- **66 组件新装边界（方案 A）已确定并自测**。当前二进制只负责新装，旧站升级和历史签到导入保留在匹配的旧二进制；旧站离线迁移另行设计。冻结历史 JSON 与指纹常数未改，测试不再用当前组件前缀生成历史清单。历史签到两条 owner 命令在连接数据库前明确返回 `CHECKIN_HISTORY_USE_PRESERVED_BINARY`。本轮升级测试 44/44、隔离 PostgreSQL 18.6 上的真实 CLI 拒绝与策略/账本不变测试 1/1 通过，Core 二进制构建、CLI 单元/真实 PG 测试串行 19/19、文档与配方检查通过。CLI 首次未配置数据库失败，随后并行新装触发测试 PG 的共享锁内存限制；串行通过，失败日志保留。此前全量 Core 记录为 1567 通过、14 失败；本轮定向复验覆盖其中 12 项升级失败与签到 CLI 用例，不把旧全量结果重标为通过。签到原失败实际是测试二进制缺失，现已构建并验证业务拒绝；image cookie 夹具的基线失败保留。导航/签到迁移、Web 与协议回归沿用本轮交接日志，整站独立验收与部署未执行。

- **尚未上线新架构**。公开服务最近核验为旧版 `0.2.118`，HTTP 200。
- **本地迁移演练**已完成身份、资金、商店/客服、订阅、OAuth/Passkey、用户组及信任基线导入读回。管理员正常激活已成功，Root auth_version从6升至7；42条密钥原版本与撤销状态保留，14条未撤销，另两个原禁用管理员保持禁用。旧raw/sk密钥共84次本地只读HTTP认证已通过，24次成功、60次按原生命周期拒绝，62张资金/计费表不变；使用合成模型目录，仅证明认证连续性。ed5fe19已修正式授权脚本owner参数化，自定义/default owner与错误owner拒绝均通过真实PG验证。
- **正式制品与固定组合**已验证。Core7 `e5b3e11`、Go9 `76aaf71`、Web5 `907197b` 的 HTTP15项、浏览器17项及release图片检查通过，采用隔离账号和本地供应商。Core7 Arch/DEB `0.1.19-7` 已构建及校验，当前提交与包未签名。
- **新依赖候选待验**。e2a64cd已将zconv升级至0.2.2、Rust对齐1.99.1；d8843d6修复空system数组生成额外消息的问题，视觉7项均通过。46adc05同步了mise、CI、原生打包、Docker构建及验收runner的Rust版本，25项针对性静态检查通过，官方Docker manifest摘要独立核对一致；当前候选的构建与完整组合验收继续进行，Core7证据保留原版本范围。
- **扩展注册与路由硬编码已更新，自测通过**。`6657c0b`、`b654171` 将11个编译包的选择、绑定、依赖与导航归属统一到注册目录，修复HeroSMS漏入启动白名单；Web接口在注册时继承插件开关和宿主准入，覆盖部署管理及内置工具等原遗漏入口。Go相关34项和完整Web兼容层138项测试通过race检查，后者5项真实PG测试跳过；Go生产包构建、Web相关8项、类型检查及lint通过。全量Go检查未通过：Unix socket受限、缺initdb，未修改的工具市场用例另报not_found。独立验收、制品发布和生产部署未执行；不能据此关闭整站验收。
- **模型配置迁移尚未完成**。共同目标的渠道凭据引用、价格版本、路由配置及发布均未落地，active release 的 config_version 为空。Source有29个渠道、23个启用；用户组21199条已导入。已生成29个凭据引用；全量规划仍因未用模型缺合同等阻塞（约592项）。来源配置673个模型；30天消费82个，其中79个仍在配置。在用79个里，计划层已有453条启用能力合同齐全（覆盖71个在用模型，含高用量文字 `gpt-6.1-sol` 与图片 `google/gemini-3.1-flash-image`）；仍卡8个在用（已书面延期，见 launch-scope-in-use-deferral.md）：Bouquet、chengzhi-c-no-flash、codex-auto-review、xingchen-agi/xing4.0-29b-a4b:free、amazon/nova-2-lite-v1、chatgpt-image-latest、gpt-image-2、gpt-image-2.5-flare。已另做 launch-scope-v2：只保留合同齐全能力、延期无合同渠道/模型并重绑哈希，得到 `candidate-requires-native-validation`（0 blockers，345路由模型）；对当前 Core 的 normal-startup 校验通过（网关/路由加载，无DB无外呼）。bootstrap_publication 已用 `PriceSnapshot::template` 修复日历价拒绝（原 `quote()` 无时间戳时隐藏 calendar tariff）；launch-scope-v2 离线校验产出 VALIDATED_NOT_PUBLISHED（401 routes，含 gpt-6.1-sol 与 google/gemini-3.1-flash-image；publication_performed=false）。已向演练库 `auth_native_target` 做 payload 约束热修补（允许 open_router / legacy_anthropic_haiku_system_cache_v1；正式 schema 指纹更新待另提交）并 `routing_control_publish` 成功：config_version=1，401 routes / 17 upstreams / 453 targets / 401 price rules；runtime 可 `routing_control_read`。Core `check-db` 对演练库报 CHECK_DB_SCHEMA_UNVERIFIED（库含68组件、0 upgrades，与当前二进制合同不一致），故旧Key HTTP 冒烟尚未跑通；payload 约束热修补与 publication 加载证据保留；空 `public_relay_preferences` 两行（ordered/disabled 皆空）需正式映射进规划器。渠道47后转换策略已编码，`test_legacy_routing_migration` 65项通过。未用配置继续保留在源归档与延期清单，不静默丢弃。
- **生产仍需完成**最终停写捕获、全阶段迁移与对账、部署切流和实际业务验收。旧快照 `FINAL_20261111T120751Z` 已因旧服务恢复写入而失去未来切换资格。
- **历史查询迁移已在本地隔离库导入并读回**。正常history阶段已提交193423条日志、20474条用量聚合、1247条用户财务投影、115笔成功充值历史及2130份用户/密钥范围回执，消费额度汇总一致；4条孤儿日志、3笔孤儿充值及14237条未导性能指标保留原件与明确缺口。最终生产快照导入及业务查询验收仍未完成。
- **历史付款责任仍在**。95笔原字段完整的充值依据已在本地安装读回；81笔缺历史字段、13笔ePay缺冻结付款快照、3笔孤儿继续保留未决权益和补证路径。60条历史订阅订单保持UNKNOWN，不能当作已自动结算。

## Rust 基础依赖审查

2026-11-12 本地改动：按领域成熟度及单一职责核对55个不同直接依赖（含本轮新增csv、移除前fs2），完整清单见[依赖审查](rust-dependencies.md)。两处CSV编码改用csv 1.4.0，保留公式防护、字段及32 MiB上限；RPC文件锁改用标准库并删除fs2，锁文件生命周期不变。zconv按用户自研最高优先级保留，未替换或关闭原协议能力。

自测：`cargo +1.99.1 test --offline --locked --manifest-path services/core/Cargo.toml --lib csv --jobs 2` 10项通过；同命令过滤`usage::export_tests`为6项通过（与前者重叠1项）；过滤`internal_rpc`为8项通过、1项因缺少DATABASE_URL失败，含socket锁定及重启的用例通过。构建使用独立`CARGO_TARGET_DIR`及`CARGO_PROFILE_TEST_DEBUG=0`。严格Clippy（`--lib -- -D warnings`）在未改动位置报70项错误；没有关闭lint或顺带修改业务。三个CSV相关文件格式通过；internal_rpc完整格式检查在HEAD已有两处差异，本轮仅删导入及替换锁调用。文档路径检查通过。没有运行真实PG、性能、独立验收、发布或部署；改动尚未提交。

### 基础编解码清理

2026-11-12：按用户明确指定，已在主树接入 `zhex = 0.1.3`，移除 hex 直接依赖；固定缓冲区解码直接调用 zhex，String/Vec 返回值仅共用分配适配层。重复十六进制编解码、百分号解码、HTML转义的替换已提交为 `d8e2541`，后两项分别使用 percent-encoding 2.3.2 和 html-escape 0.3.1。保留各入口长度、大小写、权限、金额及错误分类。zconv 为用户自研最高优先级依赖，不作热度替换；本轮未改其实现。

提交后主树执行 `cargo +1.99.1 test --offline --locked --manifest-path services/core/Cargo.toml --lib --no-run --jobs 2`，受影响的137项普通/本地异步测试全部通过，包含 zhex 分配适配和既有 HTML 表示；使用专用 CARGO_TARGET_DIR。Clippy `--lib` 通过，141条现存警告，本轮改动行无警告，不宣称严格零警告。文档检查通过。此前冻结候选的 zconv provider-controls 失败保留在本地证据；该次失败未作为本轮修复或当前 zconv 重新验收结果。未执行真实PG、独立业务验收、性能、发布或部署。SSE解析和冻结旧计价表达式解释器仍待评估，不能称全部手写基础能力已移除。


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
| MODEL-01 | 部分实现 | 文字、视觉输入、Anthropic 缓存及 Responses 结构化输出转换已有独立验证；hosted tools/fileData 完整入口与转换覆盖待核实。 |
| MODEL-02 | 部分实现 | 原生 compact 与 JSON/SSE 加密重放已通过独立验收；exact 不支持路径在扣款前拒绝，JSON/SSE 检查点绑定实际发送字节。HTTP store/previous_response_id 连续性与 WebSocket 旧行为等价仍待补；本地 GET/DELETE 资源接口并非旧版要求。 |
| MODEL-03 | 部分实现 | Chat 音频多轮历史、计费与恢复已通过独立 PG 验证；独立 TTS/STT/translation 及双向 Realtime 未齐。Gemini 音频按决定延期。 |
| MODEL-04 | 待核实 | 视频、Kling/Jimeng、MJ、Suno完整异步闭环尚无充分证据；不据旧审计推断完全无实现。 |
| MODEL-05 | 部分实现 | 渠道目录、余额探测及原生tag发布/查询/分组分页已有；tag权限、版本冲突、原子发布与重建读回的真实PG针对性自测通过。Web配置读回已保留target.client_protocols。29凭据引用已有。OAuth catalog 对日历价用 `PriceSnapshot::template`，recover ApplyPending 已消除。演练旧 Key（cred 312）：FREE/`openrouter/free` chat completed；官key/`gpt-6.1-sol` Responses 200；图片经 piqode ch8/`gpt-image-2.5-flare` 完成 generations（HTTP 200、capture 47000）。8 个在用缺口已书面延期，见 [launch-scope-in-use-deferral.md](launch-scope-in-use-deferral.md)。演练库原 8 笔 reconcile hold（约 6.51M credits）已在业主授权下以 `Usage::default()`/`Failed` 零结算释放（含图片 `failed` 无回执路径）；禁止无证据自动放款的规则不变。 |
| ASSIST-01 | 实现待验收 | Core/Go实时流、Web历史恢复与取消主要链路已实现；助手/MCP/工具/HumanGrant完整组合仍待验收。见 services/core/src/assistant_model/agent/。 |
| SUB-01 | 部分实现 | EPay一次购买的Core/Go/HTTP路径已有；其他provider首购、周期、取消及恢复逐项待核验。 |
| SUB-02 | 部分实现 | 订阅管理记录与立即失效已有；voucher、绑定删除、批量重置完整行为待核实。 |
| PAY-01 | 部分实现 | 余额/免费商店购买已通过固定组合验收；外部商店收款尚未接通。PublicConfig的platform_payment_methods/catalog为空（store/legacy_catalog.go）；external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo仍缺各自购买交付结算验收。人民币钱包充值及95笔历史充值依据已验证，属于钱包支付范围。 |
| PAY-02 | 部分实现 | 钱包Stripe/Waffo Global及受EnableLegacyRefund控制的EPay退款、商店余额全额退款已有；新订单部分退款实现见STORE-02。Pancake明确RefundEnabled=false、Refund返回ErrUnsupported（payments/pancake.go）；外部商店退款尚缺冻结交易、退款通知/查询、收入回冲及恢复全链。 |
| PAY-03 | 部分实现 | catalog/pair/save已接正常router，已有Core绑定的return URL/私钥配置可原子保存并更新运行实例；Session-only customer portal按Core用户绑定customer及单一store/environment。模块、composition、Web入口针对性测试及race通过，含并发保存、SDK本地HTTP、秘密文件持久化与配置重建。更换merchant/store/product仍需Core策略同步，当前明确返回PAYMENT_CORE_POLICY_REQUIRED；真实Core Session、PG、外部provider、浏览器及OS重启组合待验。历史缺字段订单继续在供应商I/O前拒绝。 |
| STORE-01 | 部分实现 | 匿名提货、非零价购买→邮件→提货及 PG/产品重启已独立验证；丢 SMTP 确认保留 UNKNOWN 不重发，旧订单保持原登录边界。游客购买待补。 |
| STORE-02 | 部分实现 | 新订单数量/金额部分退款及Core资金协议已有；历史余额退款分支仍不支持，完整组合待验。 |
| STORE-03 | 部分验收 | 当前固定组合购买、付款、库存提货、全额退款通过；显式卖家自购和三态目录的完整浏览器覆盖仍缺。 |
| STORE-04 | 待验收 | 账户隔离、购物车并发与清理有旧组件证据；最终组合的购物车、收藏、游客合并浏览器验收未完成。 |
| REVIEW-01 | 部分验收 | Go 已接独立模式配置、持久任务、公开文本分类、私密历史和人工覆盖。商品真实PG自测通过；工具真实PG的提交成功但ACK丢失、重启恢复旧线上版本及撤销未完成批准已独立复验通过。Core分类/签名批准、最终跨语言组合和浏览器仍待验；不据组件测试认定生产可用。 |
| APP-01 | 实现待验收 | 公开/私有GitHub配置、加密凭据、原子拉取及快照已有实现；实际Core与浏览器组合仍待验收。 |
| APP-02 | 实现待验收 | 实时用量来源、SVG与Core历史查询已有实现；固定进程组合、浏览器及公网来源仍待验收。 |
| APP-03 | 部分实现 | Go目录、报价、购买/取消、重下单、投诉、邮箱批量及管理接口已接线，5项真实本机TCP HTTP契约测试独立通过；6项既有Web API自测通过。Core直接供应商调用、报价与资金状态实现待真实PG验收；正式供应商、进程组合与浏览器未验。无provider ID的UNKNOWN保留人工对账，不能重复购买或假退款。 |
| APP-04 | 待验收 | io.net 实现及数据库权限缺陷复验通过；真实供应商、Core 授权与浏览器完整链未验收。 |
| APP-05 | 部分实现 | Core 管理、冻结报价、占用与入账消费及 66→67→68 分段升级已有 PG 验证；管理页、批量生成丢响应恢复、删除保留旧占用已通过独立浏览器/SQL 验收。provider 执行与历史优惠连续性待补。 |
| APP-06 | 部分实现 | 原生abuse策略、标记与审计已有；公开安全报告及Moderation旧行为映射仍待补。 |
| APP-07 | 待验收 | RSS 抓取、缓存、配置有组件证据；目标 edge 路由与浏览器未验收。 |
| APP-08 | 实现待验收 | 候选已有加密偏好、签名投递、持久重试和前端设置；完整Web类型检查曾通过。旧用户偏好加密续接正在补齐，真实PG/本地TLS及固定组合仍待验。旧ratio通知与投递仅有归档计数路径，未证明旧待投队列为空或已恢复，不能视作迁移完成。 |
| APP-09 | 待验收 | 展示汇率的权限、保存与账本不变有组件证据；公网汇率源及完整页面链未验收。 |
| CLI-01 | 部分实现 | 已有登录、模型目录、余额、预设及Pi适配；其他软件安装同步恢复仍有阻塞，多平台分发未齐，见 apps/cli/README.md。 |
| CHECK-01 | 部分验收 | 旧快照1247身份、883密钥、OAuth、8 Passkey、用户组已迁移核对；Root已正常激活，42条旧密钥及原禁用身份读回通过；旧raw/sk HTTP认证已通过；历史账号交互登录、实际模型配置及最终生产迁移待完成。 |
| CHECK-02 | 本地演练通过 | 旧快照钱包、欠额、推广余额、5托管、7计划、15历史订阅、60原订单及信任基线已导入读回；最终停写快照、生产对账与持续权益运行核验仍待完成。 |
| CHECK-03 | 部分验收 | 实际导入历史的API密钥只读HTTP已完成两页精确ID/总数/原始额度核对、秒级时间窗口、密钥与用户隔离、unknown金额不置零及400/401拒绝，测试前后日志/账本/审计计数不变；管理员Session、统计/财务页面、时区展示和CSV导出逐项对照仍未完成。 |
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
| 历史查询导入 | `history412-readonly-j6_q5udc/applied-01/readback.private.json`；SQL SHA256 `c14be29a8ccb7c72f398b218d6e57da7c66003e725ac053565459f9689067da2`，状态 `COMMITTED_AND_READBACK_VERIFIED`；孤儿及未导性能指标见同目录保留清单。Core7只读API密钥HTTP自测见 `history412-readonly-j6_q5udc/http-01/result.private.json`，9项通过，管理员/导出范围未运行。 |
| 信任基线 | `trust-resumed-l2yw2und/applied-01/result.private.json` 及独立readback，1247用户、104付费投影逐字段相等；首次工具版本拒绝保留。 |
| 市场审核Go恢复 | 阶段提交 `52f8776`、`6b268e9`；独立PG收据 `pg-market-review-independent-ids9wytd/receipt.json`，包含实际COMMIT后注入ACK丢失再关闭/重建连接池；129个相关源文件验前验后摘要一致。 |
| HeroSMS HTTP接线 | `6b268e9`；`herosms-go-tcp-independent.log` 的5项本机HTTP契约通过，Core资金与供应商不在此证据范围。原4项Unix socket测试因环境拒绝创建socket而失败，提升权限请求被自动审核拒绝；保留失败，不记为部署通过。 |
| 扩展注册与路由 | Web `6657c0b`、Go与开发指南 `b654171`；`go-plugin-targeted.jsonl`中modules/composition/app共34项顶层测试通过，Web第一次编译失败保留；修复后`go-webcompat-final.jsonl`为138项通过、5项PG跳过，均开启race。`go-build.log`退出0；Web8项和类型检查证据绑定`6657c0b`。后续并行i18n提交`e39c2bc`已保留，不把这些自测视为其整站验收。新增包和接口写法见[插件开发](plugin-development.md)。 |
| Core 模型与原生折扣码 | `lmm-completion-20261113/independent-core/report.json`；涵盖音频、Responses 转换、折扣码与 66→67 升级 |
| Web 折扣码报价与确认入队 | `lmm-completion-20261113/discount-independent-e924a11361/report.json`；真实 HTTP→Go→Core RPC→PG，不含 provider 执行 |
| Core 折扣码管理与 67→68 升级 | `lmm-completion-20261113/core-discount-management-68-report.md`、`core-discount-name-search-report.md`；四角色升级、数据/ACL 保留与名称搜索回归 |
| 折扣码管理页与批量恢复 | `lmm-completion-20261113/admin-browser/run-97fa78b790e7/result.json`；Core/PG/浏览器/SQL，含七语言窄屏与名称搜索；不含钱包 provider 执行 |
| 匿名提货 | `lmm-completion-20261113/independent-store/final/report.json`、`postgres-report.json` |
| 提货邮件开发自测 | `lmm-completion-20261113/store-mail-delivery/delivery-report.json`；TLS/PG/worker/race 与 SMTP 认证，不抵扣真实付费组合独立验收 |
| 付费邮件独立验收 | `lmm-completion-20261113/independent-paid-mail/INDEPENDENT-PAID-MAIL-REPORT.md`；精确 Core 基线 + mail-only Go，含实际 PG/产品重启、TLS 与丢确认后实等；不含外部邮箱/真实供应商 |
| 支付签名过期与重启 | `lmm-completion-20261113/payment-expiry-79d9d4af2e73/report.json`；真实等待 302 秒，Go 进程重启、Core 服务对象重建 |

上述路径位于操作者私有缓存/状态目录，包含受限业务材料。仓库只记录证据位置，不提交秘密与用户数据。各收据仅证明其绑定版本与范围，不代表当前源码或整站已验收。

## 范围与维护

完整要求见[产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)与[支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)。历史审计见[归档索引](archive.md)。

只更新受影响项目的当前结果、具体剩余工作和必要证据入口。源码提交、本地验收、发布与生产部署分别记录。Extore自动部署main限制继续有效。

## 2026-11-12 · owner-authorized reconcile release (launch-scope-v2 rehearsal)

- Released 8 reconcile holds on account 359 via `Charging::reconcile(..., Usage::default(), Failed, owner-authorized-zero-settle:launch-scope-v2:2026-11-12)`.
- Text×3 + image×5; image path needed `failed` operation terminal + zero-settle without receipt.
- Post-check: open reconcile=0 for those units; reserved=0; wallet restored unused holds.
| Responses 旧行为范围 | `lmm-completion-20261113/model02-legacy-behavior-review.md`；对照固定 `go-v0.2.118`，源码核对，不是业务验收 |
| 原生 compact 开发自测 | `lmm-completion-20261113/compact-freeze-r3/report.json`；绑定最终格式候选；此前开发 PG 自测 9 项与共享输入回归 5 项通过 |
| compact 独立业务验收 | `lmm-completion-20261113/compact-independent-r3-e0f8c4ed/report.json`；最终制品独立业务 4 项、共享输入 5 项通过；保留原始两项失败与修复复验，不覆盖真实供应商或生产验收 |

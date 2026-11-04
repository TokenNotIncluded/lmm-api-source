# 旧充值的未决支付义务

[资金 cutover planner](../scripts/legacy_cutover_money.py) 的钱包阶段读取本机冻结旧库，为仍可结算的充值生成安装 owner 专用的非可用资金回执。它不联系支付渠道、不创建新付款、不增加余额，也不代替支付回调迁移。

## 可结算状态不能只看 pending

旧版 `CompleteExternalTopUp` 的对应容量规则接受两类订单：

- `status=pending`；
- `status=failed`、`payment_provider=waffo_pancake`、`failure_reason_code=checkout_timeout` 同时成立。

第二类只是本地结账超时，不能据此断言支付永远不会到账。提取使用 `top_ups_payable`，不能复用只含 `top_ups_pending` 的旧导出假装完整。回执保留原 `failed` 状态、原行及金额/币种字段，`settlement_state=unknown`、`spendable=false`、`spendable_units=0`；不会把失败状态改成成功或再次给钱包记账。

旧用户已删除时，原行留在私密来源导出并计入 `unrepresented_payable_topup_orphan_rows`，不得改绑新账户、创建可用资金或标记结清。仍需单独完成 owner 隔离保留与处理决定。已有冻结阶段的输入不能原地换成新提取结果重放；完整演练必须在新库按顺序重新执行，并保留原失败和旧输入。

## 折扣名额必须关联订单生命周期

折扣码 TTL 到期不代表名额可复用。旧规则为：

- `reserved` 预留仅在对应订单明确关闭时释放；订单缺失或状态未知继续保留。
- `released` 预留若对应订单仍可结算，也占用名额。
- 没有预留记录、但仍可结算且引用折扣码的旧订单同样占用名额。
- `consumed` 已反映在 `used_count`，不重复计算。

明确关闭仅包括 `expired`，以及不属于上述 Waffo 超时恢复路径的 `failed`。可空失败原因不能凭空补成已关闭。名额核对不改变折扣定义、有效期、用户归属或最低购买额，不启用原生折扣功能。

## 本地证据与剩余门槛

2026-11-04 的冻结旧快照有 13 笔 pending 和 177 笔仍可结算的超时充值。新增本地演练保留 174 笔有映射的超时回执，3 笔已删除用户记录单独隔离；174 笔完整原行、来源 hash、owner、状态和金额均读回一致，严格重放不新增，余额及账本条数不变。旧 SQL 与 Python 独立计算均为 8 个占用名额，其中 5 个对应 released 预留、3 个没有预留记录。

私密证据位于 私有材料（本地路径已移除）。这只适用于该冻结快照、本地 owner 事务和对应源码，不证明最终快照、完整新库演练、真实到账或独立验收。

[统一核对器](../scripts/verify_legacy_cutover_rehearsal.py) 比较有映射的超时义务并保留孤儿记录、折扣容量及旧回调未验收的 gap；有缺口仍为 `SCOPED_PASS_WITH_GAPS`，退出码 2。原生折扣报价与原子预留、最后名额并发、旧签名回调的精确一次入账、晚到账消费原折扣名额，以及原支付币种/金额校验仍须单独完成接入与验证；下述实现不改变这份演练的 gap，保存回执不能代替业务验收。

## 保存逐单原结算观察值

[来源导出](../scripts/export_legacy_settlement_basis.sql) 在本机恢复库的只读事务中保存所有可结算订单、完整重基母计划及额度校准配置。`money` 按原 numeric 文本导出，避免 JSON 浮点数损失。文件含支付引用与金额，必须只存于私密备份目录。

[原函数捕获 fixture](../scripts/fixtures/legacy_settlement_basis_capture_test.go) 只用于父仓库固定历史提交 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` 的独立源码副本。将它复制到该副本的 `apps/api-go/model/`，编译测试二进制后，在禁用外网的 network namespace 内执行唯一的 `TestCaptureLegacySettlementBasisOffline`。不编译成新架构运行程序，不向新 Go 扩展提供 Core 数据库凭据。

fixture 要求本机 私有材料（本地路径已移除） 下的 socket、端口 55436、名称以 `lmm_legacy_quote_oracle_` 开头的专用旧库副本。`LMM_LEGACY_ORACLE_SOCKET`、`LMM_LEGACY_ORACLE_DATABASE`、`LMM_LEGACY_ORACLE_EXPECTED_ROWS` 和 `LMM_LEGACY_ORACLE_OUTPUT` 必须显式提供；输出文件不可已存在。它调用原 `VerifyCreditUnits` 的只读路径、`pendingTopUpSettlementQuotaTx` 和 `expectedTopUpAmountMicros`，随后回滚观察事务。仅原 `ErrInvalidTopUpQuota` 计为拒绝，存储等其他错误必须失败，不能伪装为业务拒绝。保留完整源码、捕获 fixture、二进制、运行日志、来源与副本一致性证明及指纹。

[事实 preparer](../scripts/prepare_legacy_settlement_basis.py) 接受 `--input`、`--oracle`、`--archive-sha256`、`--expected-rows` 和私密 `--output`。oracle 文档绑定原提交、二进制/捕获源码/原源码归档 SHA-256、完整表与原用户 ID 向量 hash、禁网与回滚范围，以及逐单原结果。preparer 验证精确覆盖、owner、金额和拒绝状态；这些 hash 是来源保留材料，不是对任意 JSON 的真实性签名，仍需独立审核原函数执行过程。

生成的 `legacy-topup-settlement-facts.v1` owner 回执绑定同一 opening archive 与观察时间，严格重放只接受同一 manifest。它保存原订单引用、原额度与原应付金额、被拒绝记录和未映射用户；`runtime_settlement_available=false`。当前运行时依据表为 `core_payments.legacy_bases`，见下节；它与事实回执是不同层次。下述离线 preparer 可生成依据安装 SQL，真实渠道审核与审批执行尚未完成，不能把事实回执当作付款或发放授权。

本次固定快照的原函数观察为 190 单中 182 单额度有效，8 单 EPay 记录被原规则拒绝。全部订单的原应付金额可读；3 单 owner 已删除。逐单观察与订单、1 份重基母计划、5 项额度配置均已在本地回执精确保留，重复导入无额外资金写入。独立业务验收与真实回调仍未执行。

### 新旧订单算术边界

preparer 额外输出 `compatibility.private.json`，只核对当前新订单的整分金额与整数 `credits_per_minor` 约束，不修改原 manifest 或冻结报价。原快照的 182 笔有效报价中，3 笔应付金额不是整分；其余整分订单中有 177 笔的原额度无法表示成整数 credits/分，仅 2 笔算术上可表示。该数字不证明渠道、验签、归属或退款就绪。

旧回调需要独立的冻结报价结算依据，保留原金额的微单位精度和原有效额度。不能通过舍入原金额、调整到账额度或重选当前汇率适配新订单表。当前报告始终为 `native_intent_replay_authorized=false`，实际运行时结算仍未开放。

### 旧 Pancake 回调验签

Core 的 [独立验签器](../services/core/src/payments/legacy_pancake.rs) 接受 owner 审阅后绑定的原订单引用、币种、微单位金额和原有效额度。它复用原始字节 RSA 验签、45 分钟重试窗口及重复 JSON 键拒绝规则；不使用新渠道的 credits/分比例或新订单金额上限重算旧报价。币种为空的旧行仍需 owner 显式绑定，不能从当前渠道配置推定迁移已经就绪。

金额保留原十进制及科学计数法语法，换算结果必须为正整数微单位，不舍入；尾随零不改变金额。与固定旧 SDK 一致，`chargedAmount` 优先于目录 `amount`，带 `listPrice` 却没有实付金额的现代报文不能证明付款。老报文允许缺少产品/商户元数据；有原产品绑定且报文携带产品键时必须匹配，原产品不会被新渠道产品覆盖。原钱包规则不校验 `lmm_merchant`，legacy 分支同样不把该字段与当前 merchant 比较；渠道归属仍需 owner 绑定。新订单的整分金额和元数据要求保持原规则。

原 Go 将 `data.orderId` 保存为供应商交易号，新 Core 则使用 `data.paymentId`。验签结果分别保留这两个标识、原 event id（缺少时回退 eventId）、原金额、原额度和载荷 hash。旧报文可以缺少 paymentId；这不代表允许绕过跨新旧订单的交易去重。当前共享标识表和入账存储已处理这两种标识的归属与并发约束，见下节；这不代表回调接入已完成或验收通过。

该函数只验证回调观察，不创建付款证明、折扣消费或账本记录，接入范围见下述内部 RPC 与回调入口。原始事实回执仍不可授权发放。当前自测使用本地生成的 RSA 签名，不是供应商真实回调或独立验收。


## 当前 component 50/51 内部实现

以下描述内部实现及其局部自测，不代表发布、完整业务独立验收或生产就绪。[schema](../services/core/schema/legacy_payments.sql) 保留 fresh component 50 原文；[component 51](../services/core/schema/legacy_payment_opening_identity.sql) 显式修正 opening identity。当前只支持 fresh 51，不提供从已有 49/50 升级到 51 的流程。普通启动不能代替 owner 安装或既有站点升级。

### owner 安装的不可变依据

`legacy_bases` 保存原订单、原用户、来源归档与原行 hash、审核 hash，以及绑定的商户、环境、渠道、币种、store/product 和原折扣码。插入触发器要求依据的 opening migration、显式原 opening entity key、来源归档 SHA-256 与原 opening 导入一致，且目标必须是该映射用户的原个人账户。不能改绑团队账户或其他用户，也不能为已删除用户凭空生成归属。依据仅接受 pending 或 checkout_timeout 的 failed 原订单；冻结的正整数 `amount_micros` 和 `credit_units` 保留原微单位金额及原有效额度，不按当前价格、汇率或 credits/分重算。

依据表禁止更新、删除和 truncate，运行时不得写入 owner 条款。上述约束不验证审核材料的真实性，也不执行 owner 审批；下述加载器只生成安装材料，没有执行真实旧订单依据安装或审批；合成依据仅用于本地自测。原函数 oracle、来源归档及独立审核要求仍然适用。

component 51 将原生成列变为不可变显式 `opening_entity_key`，仅接受与原用户 ID 严格相符的 `user:<ID>` 或 `legacy.users/<ID>`。前者来自 `legacy_cutover_import.plan_opening`，后者来自独立 financial planner/preflight。FK 和 guard 匹配审核指定的精确标识，不默认猜测、不改写 opening，不回退另一种形式。v2 审核文件必须明确该键；既有 v1 材料须按实际 opening 重新审阅，不能直接安装到 fresh 51。

### 离线依据准备与安装

[绑定 preparer](../scripts/prepare_legacy_payment_bindings.py) 复用上述 source/oracle 校验，接受相同的 `--input`、`--oracle`、`--archive-sha256` 和 `--expected-rows`，以及新的私密 `--output` 目录。未提供 `--bindings` 时只输出 `bindings.template.private.json`：真实冻结快照产生 174 笔可审阅草稿；被拒绝的原额度、已删除 owner 和不支持的供应商保留在 gaps，未选订单不自动激活。草稿中的渠道和目标账户仍需审核补全，没有安装 SQL。

提供 `--bindings` 时，文件仅含 `scope=owner-reviewed-legacy-payment-bindings.v2`、原 facts 的 `facts_sha256`、`opening_migration_id` 和明确选中的 `orders`。不能直接把带有 draft scope、gaps、installable 等报告字段的草稿作为审核文件。逐单绑定保留原行 hash，显式提供原 `opening_entity_key`、目标 user/account/channel、merchant/environment/currency/store 和原 product；原非空币种、store/product 不可用当前配置替换。金额和额度只取原函数观察值，不能由审核文件重写。审核 hash 按行绑定原 facts 与 opening identity，后续增加其他审核行不改变旧行。

输出 `plan.private.json` 和 `install.private.sql`，均以 0600 排他创建，目录须禁止组和其他用户访问。SQL 要求实际安装 owner、已安装且完全一致的原 facts、同一来源归档和观察时点的 opening run，再依靠账户归属触发器逐单验证。依据安装和严格重放在一个可持久化的 serializable 事务中完成；同 ID 内容冲突整批失败，不改写旧依据。安装只增加不可变依据，不联系渠道、不产生付款证明、不调用账本发放。

本地自测使用真实新库验证安装、重复安装、增量选单、缺失 facts、原账户不匹配整批回滚、冲突审核重放和非 owner 拒绝；没有安装真实旧订单。显式隔离的新库可通过 `LMM_BINDING_TEST_DATABASE_URL` 运行 [集成测试](../scripts/test_prepare_legacy_payment_bindings.py)，数据库名必须以 `lmm_binding_test_` 开头，目标仅为 `127.0.0.1`；测试需要当前 Core 二进制、psql 和可创建 NOLOGIN 测试角色的安装 owner。默认未设置该变量时只运行纯 preparer 用例，PostgreSQL 用例标为未运行。

v1 独立 owner 使用另一组合成输入，通过真实 CLI 和全新 PostgreSQL 验证了 preparer / SQL loader 的文件权限、字段原值、严格重放、整批回滚及 facts/opening/owner 拒绝；账本与余额全量快照不变。该 v1 局部验收不包含原 Go oracle 执行、真实旧订单、付款运行时或完整迁移。后续联合流程揭示了两个官方 opening 标识的差异；v2 和 component 51 已修复，并通过同库混合两种标识的本地回归，独立主进程复验已覆盖 v2 官方 financial opening 路径，范围见下节。

### 接纳证明与资金应用分开

[存储实现](../services/core/src/payments/legacy_store.rs) 的 `admit_legacy_pancake` 只读取已安装依据，匹配配置中的 provider、merchant、environment 和 currency，再按冻结报价验签。它持久保存签名原文、签名、载荷 hash、event、供应商 ORD（orderId）及可选 PAY（paymentId）；不会增加余额或写入资金账本。同一 event 仅在依据和载荷 hash 一致时返回原证明，不同归属或内容冲突会拒绝。同一依据已有证明的 ORD 不能改变，两个非空 PAY 也不能互换。

[共享标识实现](../services/core/src/payments/identifiers.rs) 将 native intent 与 legacy basis 放入同一 provider/merchant/environment 范围的标识空间，按 `event`、`order`、`payment` 分别保留 event、ORD、PAY 的类型。每个标识只归属一个原支付主体；native 的 credit 接纳也认领 payment，Pancake 同时认领 ORD。缺少 PAY 的旧报文仍认领 ORD，不能借此跨新旧主体重复使用交易标识。类型化标识不是把任意同名 ORD 与 PAY 文本视为相同值。

`apply_legacy_payment` 单独应用持久证明，以 basis 事务锁、按 basis 与 operation key 派生的稳定 operation id、账本操作及 `legacy_applications` 同一事务保存结果。schema 对每个 basis 只允许一条 posted 应用，并校验 journal 的 scope、操作号、原个人钱包与冻结额度。本地 PostgreSQL 自测覆盖该精确一次应用机制，独立端到端回调验收尚未完成。提交结果不确定时必须使用同一 operation key 重放；同一 basis 与 key 返回原持久回执；后续已验签的 event/PAY 证明不会改变该资金请求，仍返回首次 proof/outcome，不重新记账。basis 一旦 posted，后续新 key 或后续证明返回首次 posted 的原 operation/proof/outcome 回执，不新发资金。rejected 回执也保留；核对后使用新 key 才能重试，不能改写原拒绝结果。

### 完整折扣事实前置材料

统一离线导入器新增 `--phase discounts`，注册在 Money 阶段之后、history 之前。
`legacy-discount-facts.v1` 回执保存全部折扣定义、预留、关联订单、报价单位白名单配置及逐订单占用来源。单位配置白名单同时包含 `general_setting.quota_display_type` 和旧兼容字段 `DisplayInCurrencyEnabled`；后者也会改变旧运行时展示与门槛单位。冻结源中两字段均缺失，按缺失保留，不填入当前默认值。原优惠码按旧规则去空白、转大写后存 SHA-256；其余定义字段，包括私有 owner、原 used count、门槛、时间与软删除状态均保留。完整原始表摘要另外保存，不能用当前政策改写旧条款。

本机 component-51 目标库的实际导入为 98 个定义、16 条预留、28 笔关联订单和 8 个占用事实。5 笔仍可支付的超时订单即使已有 released 预留也保持占用事实，另有 3 笔可支付订单没有预留行。预留 TTL 本身不能证明订单关闭；缺少订单或未知失败原因也不释放仍为 reserved 的记录。

导入沿用安装 owner、opening 归档与 observation、用户映射和不可覆写回执检查，由外层统一 runner 提交 SERIALIZABLE 事务。回执明确记录 `balance_effect=none`、`native_discount_enabled_by_import=false` 和 `capacity_reserved_by_import=false`；它不修改账本或旧 used count，也没有创建原生折扣预留。通用 owner SQL body 支持嵌入此事务，并选择载荷中未出现的 dollar-quote 分隔符，保留名称中的引号及类似 SQL body 标签的文字。

统一只读核对器逐行比较脱敏定义、完整预留、关联订单、单位配置、来源绑定及逐订单占用。定义和预留的事实比较通过后，可消除其“未覆盖”统计；`discount_capacity_holds_not_imported` 继续表示尚未生效的原生容量保护，并另保留原生报价、预留和退款未验收门槛。保存这些材料不能代替下面的 Core 接入工作。


独立实际 PostgreSQL 验收通过严格重放、篡改拒绝、错误 opening 来源拒绝、未切 owner 拒绝、非空 scope 拒绝，以及含 body 标签和引号的名称原样回读。之后另建全新 component-51 目标，按注册顺序运行 16 个阶段及 4 个补充导入；独立逐阶段回读、目标 Key 别名绑定和统一 30 项对账通过。所有 233 张 Core 表在只读复验前后保持一致，状态仍为 `SCOPED_PASS_WITH_GAPS`。该演练不包含 Go 业务数据的完整安装、原生折扣、全页面验收或上线。

### 折扣消费视图与未完成接入

`legacy_discount_consumptions` 仅投影已 posted 且带原折扣码的依据，以来源归档和原订单标识消费。后续名额计算须从导入的 held 集合扣除这些订单，并将消费加入原 used count，避免同时计为预留和已用。该视图不是完整优惠券引擎，不实现原生报价、原子预留、最后名额并发或完整折扣生命周期。

### 内部 RPC 与旧订单回调

内部 `CorePayments` 增加 `AcceptLegacyProviderProof`、`GetLegacyProviderProof` 和 `CreditLegacyTopup`，分别使用已有的 proof ingress、proof read、credit evidence 服务权限。服务令牌、protocol metadata、并发上限和超时沿用支付 RPC 边界；客户端不能指定到账账户、汇率或金额。证明响应只含持久 proof/basis、原订单、事件、ORD/PAY、载荷 hash 和接纳时间，不返回签名或到账状态。恢复查询绑定当前配置中的渠道 namespace、原订单、事件和精确载荷 hash，不重新验签；合法公钥轮换后仍能恢复已接纳证明，未知或变更内容不能凭查询取得发放依据。

Go 的原 `/callbacks/{channel}` 为 Pancake 增加旧引用分支；`top_` 前缀仍走新订单路径，失败不能回退到旧订单。冻结快照的 177 个 Pancake 原引用均不匹配新订单格式。旧分支保留原始请求字节与签名，不把原微单位金额转成新订单整分，不强制匹配当前 Go 商品元数据。它先查询持久证明，仅 NotFound 才提交验签；随后使用固定 `legacy-credit:<basis_id>` 操作键请求入账。只有 Core 返回有效 CREDIT_APPLIED、原 basis 和持久 journal 才确认成功；rejected、未知提交和其他错误不确认成功。新的 delivery/proof 若对应已入账 basis，Core 可返回首次入账的原 proof 回执。

新入口没有安装真实旧订单依据，仍需真实渠道绑定审核、完整折扣/返佣等副作用和最终业务验收。当前没有自动扫描并应用遗留证明的恢复 worker；保留的证明可在后续回调时恢复，不能把仅接纳证明当作已完成入账或已排队资金。

本地自测已用隔离 PostgreSQL 验证新库安装、证明不增余额、按原额度入账、原回执重放、后续 PAY 别名、单次折扣消费投影及新订单 ORD 冲突拒绝；原有 14 项渠道测试、两项运行时最小权限与禁止写权限测试通过。测试使用本地 RSA 签名和合成账户，Core RPC 方法已在真实数据库验证，Go 回调及 bridge 包测试通过；独立真实 Core/Go 主进程及分离权限 PostgreSQL 联合验收 13/13 通过：正常首次回调、立即重复、证明已保存但无入账时重启、过期/畸形签名配原载荷恢复、新 event/PAY 别名及相应拒绝分支。两笔合成订单各只有一个 application/journal，钱包精确为两份冻结额度之和；原回执内容不变。验收使用冻结二进制，没有重新生成编译缓存，不覆盖真实供应商或完整折扣引擎。Rejected 回执的 HTTP 重放、原生新订单完整生命周期、自动恢复 worker、最终全量演练及页面验收仍未完成。尚无真实供应商回调，也未执行独立业务验收；上述历史本地演练及 RSA 自签名验签结果不能代替这些门槛。

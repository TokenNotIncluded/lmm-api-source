# A04 site-statistics 独立合同核对

状态：只读源码合同核对完成；不是数据库、实际 Native、升级、浏览器或真实历史验收。独立预期值来自下列固定旧合同和人工定义的合成事实，不调用未来开发 reader/聚合函数作为 oracle。

## 固定来源

- 父仓库发布基线：`860e50d2e21ad51deceac5c74f9a8068fceb9ad1`。
- 父仓库 `origin/traditional` 与 `origin/传统` 均指向 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`。只有远程引用；旧代码全部用父仓库 `git show <固定SHA>:<path>` 读取，没有 checkout。
- 新源码基线：`e3b1f1db0e23884e0f84ebe366e414528daca565`。新源码同样用 `git show` 读取，未纳入并行 WIP。
- 旧 UsageProjector 实际文件名为 `apps/api-go/model/usage_projection.go`。
- 归属：旧实现 copyright LIghtJUNction / AGPL-3.0-or-later；本文引用其语义，不移植产品实现。

## 旧公开 API / DTO 的精确含义

`GET /api/finance/site-statistics`，无时间窗/分页合同，全保留账户累计和当前快照。路由 `AdminAuth + DisableCache`；handler 再校验 admin/root。成功包为 `{success:true,data:{...}}`；读源失败 503，不能返回伪造零。只读事务使用 PostgreSQL REPEATABLE READ（SQLite SERIALIZABLE），金额与订单在一个快照中读取。

| DTO 字段 | 旧合同 |
| --- | --- |
| `as_of` | 整数 Unix 秒，旧实现读取结束时的时间标记；不证明某历史来源已完整导入 |
| `credits_per_usd` | 固定整数 500000；是公开 credits 显示合同，不是历史货币换算证据 |
| `total_balance_credits` | 所有保留 User 的当前 `quota` 之和；含停用/软删除用户；允许负数；十进制整数字符串 |
| `total_used_credits` | 所有保留 User 的 `UsageProjector.User(...).NormalizedUsedQuota` 之和；非原始 used、非逐日志金额、非钱包入账总额；非负整数字符串 |
| `recharge.currencies[]` | 按标准化原订单币种分组，排除 LDC；币种按字典顺序排序 |
| `recharge.virtual_units[]` | 旧合同仅 LDC，独立于现金列表；不得合并成 USD/CNY |
| `gross_amount_micros` | 成功且符合来源条件订单的原币种 settled 微单位合计，非报价、赠送额度或兑换后金额 |
| `refunded_amount_micros` | 同一批订单的累计已记录原币种退款；不能再叠加退款事件表而重复计算 |
| `net_amount_micros` | 同币种 gross-refunded；全额退款仍保留该订单，net=0 |
| `orders` | 当前币种的 confirmed 订单数，不是事件数/付款次数/充值日志数 |
| `confirmed_orders` | 现金与 LDC 两类有效订单数之和 |
| `unconfirmed_orders` / `invalid_orders` | 下文分类得到的订单数；不是所有 pending/failed 订单数 |

金额字符串总和需要任意精度整数或 PostgreSQL numeric 后输出十进制文本；不能经 f64、JS Number 或最终 i64。单行安全域不能限制全站总和为 i64。当前 Web schema 要求金额非空字符串、不接受 null；余额允许负，used/付款不允许负；count 是 JS-safe 非负整数。显示用 BigInt，付款微单位保留至六位小数、最少两位。未知若采用 nullable/availability 扩展，必须同步 Web 解析和空/错误状态，不能只在旧 DTO 旁加一个会被忽略的 partial 标记。

单位：1 原币种单位 = 1,000,000 micros；Native USD `amount_minor` 是 cents，所以 `micros=minor*10000`。`credits_per_minor=5000` 只约束 Native Stripe/USD 的入账比例；不能推广到历史折扣订单、CNY、LDC 或任意 ledger credits。钱包显示为 USD 的估算也不是用户现金实付。

## 旧充值分类：顺序是合同的一部分

依据旧 `model/admin_site_statistics.go` 与 `model/topup.go`：

1. 仅选 `top_ups.status=success`。同 `trade_no` 只要存在任意状态的 `subscription_orders`，排除充值镜像；不要求订阅 success。不是新账户 active 过滤，也不 join User 排除历史订单。
2. method/provider trim+lower；currency trim+upper，语法必须正好三个 ASCII 大写字母。旧语法不是 ISO 币种目录校验，不能擅自把合法三字母未知币种按汇率换算。
3. LDC 特例：currency=LDC、provider=epay，且 `isLegacyLinuxDOCreditTopUp` 成立。该 helper 接受 ldc/linuxdo/linux_do/linuxdo_credit；method=epay 时按其原币种/快照条件判断。
4. 非上述 virtual 且 `IsFinancialPaymentSource` 为 false 的订单直接忽略：balance method/provider，以及 gift/bonus/checkin/invite/bounty/linuxdo/linux_do/linuxdo_credit/internal/admin 等原黑名单。商店收入/手工财务流水根本不在 top-up 查询范围。
5. virtual 缺不可变预付款快照（expected>0、credited>0、currency 非空），或非 virtual 是 LDC/不属于 knownExternalTopUpSource：unconfirmed++ 并跳过。允许的 provider 是 epay/stripe/creem/waffo/waffo_pancake；provider 空时 method 只接受 stripe/creem/waffo/waffo_pancake/alipay/wxpay。非空未知 provider 不能靠 method 冒充已知来源。
6. 然后才检查 settled<0、refunded<0、refunded>settled：invalid++。
7. 然后检查币种语法非法或 settled=0：unconfirmed++。
8. 其余按现金/virtual 分组累加 settled/refunded，orders 与 confirmed_orders 各+1。

因此“未知 provider 且负金额”是 unconfirmed，不是 invalid；“已知来源、币种缺失、退款超过 settled”先命中 invalid。expected、Money float、credited_quota 均不得补出实付。旧结果允许确认子集金额，但通过两个未知计数明确说明排除项；这不等于允许把未导入的整个历史集合当作 0 个未知订单。

## 历史使用量与 opening 的独立公式

旧 `usage_projection.go:67-159,162-247`：没有该用户 rebase baseline 时，旧实现把其 current_used 作为当前单位用量；若存在基准，必须验证 version/kind、完整历史标记、500000 anchor、正且>1的精确 decimal divisor、合法 rounding、完整且不重复的 user/token scope，以及历史 fee 冲正审计。

设 B=原始 baseline used、C=当前 raw used、D=冻结 divisor、Rraw/Rnorm=有证据的历史冲正累计：

```text
historical_normalized = round(B / D, frozen_rounding) - Rnorm
post_migration_delta  = C - B + Rraw
normalized_used      = historical_normalized + post_migration_delta
```

rounding 只能使用来源所载 half-away-from-zero 或 toward-zero；不能使用当前 K/FX。负 delta、缺/坏审计、不明冲正、越域均 UNKNOWN/失败，不能夹为0。逐 log 的 captured pricing basis 与这个累计用户 projector 不是同一个合同。

新 `account_opening_counters` 只有 raw used/request_count/aff_history 及来源 hash，request 的 SQL 白名单也没有归一化基准。`financial_profile.rs:241` 仅 opening_used=0 才给 normalized=新 settled-refunded，否则 None。原始 used 甚至允许带符号安全域；不能因 schema 接受便宣布非负规范用量有效。

`opening_imports` 证明某账户 opening wallet/reserved/affiliate 以及来源身份，不证明历史收入、归一化使用量或全站源集合完整。不能重复加 opening 到已经包含 opening entries 的 balance_state。新 `financial_profile` 只服务活跃个人用户，不能把其活跃过滤直接搬到全站查询。

## e3b1 持久化事实与可复用边界

| 来源 | 能证明的事实 | 不能证明/缺项 |
| --- | --- | --- |
| `core_billing.ledger_accounts + balance_state` | 当前 wallet、reserved、debt、affiliate，各自整数；个人 quota=wallet+signed debt，reserved/affiliate 单列 | 不等于现金；不能求全 ledger 和（双重记账通常合计0）；全站需保留停用/删除映射与完整账户集合。团队/公司若另计需显式版本语义，不能混入旧 User 总额 |
| `opening_import_runs/imports/account_opening_counters` | archive/mapping/row hashes、原账户 opening、raw 累计计数 | 无旧 UsageProjector divisor/冲正/归一化结果证明；没有全站已导入覆盖 seal |
| `core_charging.requests` | 每逻辑 request 的 settled-refunded，state=settled；可独立得已接纳新计费净用量；actor/payer 不混同 | 不证明旧 opening used；不能宣称覆盖其他非 request 费用/历史用量。不能再把 matching ledger capture 或 performance 事件加一遍 |
| `core_payments.intents` + receipt/provider evidence | 仅 Native Stripe/USD；不可变原金额 cents、credit_units、provider_transaction、credit_journal_id、累计 refunded_minor | prepare 行本身不是已付款；金额 quote 不等于入账成功；没有历史 CNY/LDC 等来源 |
| `accepted_proofs` | Rust 已验签并绑定 intent/channel/merchant/environment/金额的 Credit/RefundSucceeded 等事实 | 只代表验签输入被持久化，不代表资金状态机已完成；Pending/Failed 不是成功退款 |
| `provider_refunds/refund_holds/blocked_refunds` | refund ID 去重、已提交退款、待处理退款/冲突事实 | held/released 不应作已退款；不能忽略 blocked，把 refunded_minor 当完整外部退款总和 |
| `core_history.sources/log_records/scope_receipts` | 已装历史日志范围与逐 log pricing basis | 不是全账户累计用量或历史 top-up/订阅排除/原币种退款档案 |
| `core_trust_pricing.legacy_paid_orders` | 剩余 net credits、paid_at、source digest，用于价格资格 | 没有原币种 gross/refund；net_credits 不可反推钱；完整价格资格导入不等于完整财务观测 |
| `migrate-legacy-financial.py` 的 topup_history/refund_lineage | 离线计划保留原币种、原实付/退款等行事实与 hash，可作为准备新观测来源的输入之一 | planner 是 dry-run，不是 runtime 已安装表；`financial_import.py` writer 只写 opening/counters/可选 escrow，不导入整个 topup_history |
| `core_migration.import_runs` 的 wallet-topup-history.v1 | 已有 owner-only 个人历史快照载体/完整行数与身份映射 | 当前 `prepare_wallet_history_snapshot.py` 丢失 payment_provider、settled/refunded/expected、订阅镜像排除依据；money_text/平台金额不能替代原始实付。canonical runtime grant 也未授整个 import_runs SELECT |

Native 已信用入账的正常 USD 样本可以由 credit_journal_id + 已验签 Credit/原始绑定证明，并以 refunded_minor 和去重退款行交叉核对。必须另列“全额退款先于 credit”“已验签部分退款但资金恢复被阻塞”等分支：现 `payments/mod.rs:1275-1288,1392-1409` 确实允许无 credit journal 的退款事实/围栏；简单 `WHERE credit_journal_id IS NOT NULL` 不等价于全部已发生外部付款。对未解决的分支应显式 UNKNOWN/待对账或独立展示，不能从总表静默漏掉并宣称全量。

## 财务读权限与数据库权限

旧 finance 路由用静态 AdminAuth，不受九项动态渠道权限覆盖。新 history route 目前经 `authenticate_category(Administrator)` 调共享 `authorize_admin_read_snapshot`。实际 helper 读取当前个人 Session、role、auth_version/active/MFA 等认证事实，要求 admin/superadmin；导入管理员还须完整且 hash 匹配的 policy。当前内部使用 AdminUsersRead/Directory；`FinanceAuditRead` 与它在 permissions.rs:660 都是静态 admin/root 角色规则，并非 catalog override 动作。不要把此前 ChannelRead deny 测试的403原样套给财务读。

后续独立矩阵：有效 native admin/root；完整导入 admin/root；普通用户、API-key、非个人账户凭证、匿名、过期/撤销/版本旋转；导入 policy 缺失/不完整/hash mismatch。预期按已定义身份错误区分401/403，存储不可用503。财务授权不授资金写入或源导入能力。

SQL 权限与 HTTP 权限分开：canonical grant 对 core_billing 账本/开账来源有 SELECT；history 表 SELECT。payments 全写权限仅 enable_payments 分支，enable_trust_pricing 只补 intents/requests/usage_events 读，不能因此假设所有 proof/refund 表可读。新统计 reader 应获得明确的最小 SELECT，不能通过打开支付写权限解决读依赖。owner-only 新观测及其 sequences/functions 应验证 runtime 无 INSERT/UPDATE/DELETE/TRUNCATE/owner/DDL/导入执行权限；公开读端点不应读取含秘密的整份迁移 manifest 或 private core_meta receipts。

## 独立合成预期值（未执行，非作者 fixture）

这些数值先于未来实现冻结；fixture seeder 只写明确合成事实，oracle 用整数/有理数和下列手算式，不 import 开发实现。

**用量与余额**：B=1205，D=2，half-away-from-zero；已证冲正 Rraw=200/Rnorm=100，C=1155。历史归一化=603-100=503，迁移后delta=150，切换时 used=653。新请求 settled70/refund10 再加60，另一原生用户使用42，则全站 used=755。若来源 rounding=toward-zero，结果754。删除基准/冲正证据或使delta<0，完整 used 必须 UNKNOWN，不返回755或0。

余额用户A wallet100/debt(-30)/reserved20/affiliate700；停用B wallet40；软删除C debt(-5)。旧兼容当前余额=70+40-5=105，reserved/affiliate不加入，不能过滤B/C。这个向量假定 source mapping 已证明这些账户对应全部保留旧 User，不能用它自行证明真实覆盖。

**充值**（均成功钱包订单，除单列排除项；单位均 micros）：

| 合成行 | 来源与币种 | settled | refunded | 预期 |
| --- | --- | ---: | ---: | --- |
| A | stripe/USD | 1200001 | 200001 | confirmed，net1000000 |
| B | creem/` usd ` | 800000 | 800000 | confirmed，全退仍计1单 |
| C | epay/alipay/CNY | 2300005 | 3 | confirmed，net2300002 |
| D | epay/ldc/LDC，expected3300000、credited9000000 | 3300000 | 1000001 | virtual confirmed，net2299999 |
| E | 非空 unknown provider/USD | -1 | 0 | unconfirmed（来源检查优先） |
| F | stripe/空币种 | 1 | 0 | unconfirmed |
| G | stripe/USD，expected999999但settled0 | 0 | 0 | unconfirmed |
| H | epay/ldc/LDC，缺expected快照 | 4 | 0 | unconfirmed |
| I | stripe/USD | 5 | 6 | invalid |
| J | stripe/USD | -2 | 0 | invalid |

结果：cash按CNY、USD排序；CNY=(2300005,3,2300002,orders1)，USD=(2000001,1000001,1000000,orders2)；LDC=(3300000,1000001,2299999,orders1)；confirmed4/unconfirmed4/invalid2。再插入gift、balance、pending、failed、商店收入、以及有pending订阅镜像的成功topup，结果应不变。退款事件重复不改变累计金额；变更credited/expected/Money也不改变A/B实付结果。

另测 Native已确认USD123cents、已确认退款23cents：gross1230000/refund230000/net1000000。一个仅prepare的999cents行不可入confirmed；blocked退款不能被误认为0。Native与历史重叠源ID/交易映射必须只计一次。1031行各9007199254740991的全站整数合计必须精确为9286422431637961721（超过i64；直接整数乘法确定，不调用产品聚合器）。

## 完整恢复所需最小来源与验收边界

主线程拟补 owner-only 完整财务观测来源与升级的方向能承载缺失语义。独立要求是：

- 版本化 scope/来源命名空间、archive/mapping/逐行hash、cutoff、完整保留用户/订单集合及显式空源证明；不可拿 opening 行数或日志complete替代全量 seal。
- 每用户保留 raw used、审计归一化所需 baseline/divisor/rounding/冲正，或由已冻结算法独立复核且保留原证据的 normalized opening；保留停用/软删除映射。旧剩余余额是存量，不是历史现金总流量。
- 每历史订单保留 method/provider/status/原币种/settled/refunded/expected/credited 与必要订阅镜像映射；缺字段保持缺失。退款有原订单/原币种/唯一来源映射；不重新信用入账。
- 明确切换边界及 native lineage，证明历史与新事件无重叠/漏项；退款跨边界不能二次扣减。无法证明全量时分别输出已知范围与未覆盖，不称历史恢复。
- owner-only installer/升级与 runtime SELECT 合同同时落地；来源可独立读回、完整性可核对；不是仅放 schema 或改503。来源缺失/不完整/有毒单位必须阻止可信全量金额输出。
- Reader 单一 REPEATABLE READ READ ONLY 快照，来源验证、鉴权、所有聚合/as_of 一致；聚合前后受保护表与sequence不变；保持资源时限，超限不能成功返回隐形截断总额。
- Web明确全量/部分/UNKNOWN，保留精确金额、原币种、LDC、未知分类计数；不把不可读/缺表当空站。

仍未验：新schema/upgrade/owner import、真实旧来源完整性、恢复后的实际Native HTTP/PG/ACL、并发快照与大数资源边界、导入幂等与COMMIT UNKNOWN处理、Native付款/退款异常矩阵、Web浏览器。本轮没有编译、安装、启动数据库、调用支付或访问生产/DMIT；不以此文档签业务PASS。

## 可复现只读命令与源码摘要

在父仓库运行 `git show 3948a0a1ee1cefdd016a7aa45a11851545bb4b85:apps/api-go/model/admin_site_statistics.go`；在source仓库运行 `git show e3b1f1db0e23884e0f84ebe366e414528daca565:services/core/src/financial_profile.rs`。其余路径同下表；无 checkout。SHA256 对象是对应 git blob 的原始内容，非工作树。

| 树 | 路径 | SHA256 |
| --- | --- | --- |
| old | `apps/api-go/model/admin_site_statistics.go` | `5ed495e15ab0f04b393a8503854a686e16b1c0860b937dea8568ca8fe6aa95e8` |
| old | `apps/api-go/model/usage_projection.go` | `ad7e4bd0ac0af7a0a06d4d77fd234997c09e97213aea6694af8dabf3317b66c1` |
| old | `apps/api-go/model/topup.go` | `39f7247c6d5d57f0cef008f2117f72d524e391e5b8c628657f03ace2c1a9be8e` |
| old | `apps/api-go/router/api-router.go` | `cc7d608a4cfd94766090d57f45624822da080784e00b496bb32f5e6dd6447346` |
| new | `apps/web/src/features/dashboard/site-statistics.ts` | `8d267270a7cce1298e1c82ce2a0f8358a926f2507104e7bce4f0230f4aae2e33` |
| new | `services/core/src/financial_profile.rs` | `fef3f3ab5d1fdbe6dd57d0a97995412f6c0003c180853ff9af376836352ed053` |
| new | `services/core/schema/ledger.sql` | `ce973f6fee25904d4fcdc87895ec49e6c53e10c9fb3508df4d7d72ace16df77f` |
| new | `services/core/schema/payments.sql` | `9579afddbe5aeb4deb67984e731bcdc6ca405dc619ceede8d92505ecc43f7d39` |
| new | `services/core/schema/trust_pricing.sql` | `fcb463206c58b52b0bddb7c33051b79efc5df36c10b29773c4895069fb17ee95` |
| new | `services/core/src/payments/mod.rs` | `12d138e6d164925caf58b75389c5df2830ede4d40ef6782f34f08a2a8545301c` |
| new | `services/core/src/history_read.rs` | `05cdf4a8a2e28e54b82b76d975c9119400a107b74365cf054b18ebf00bc4ccb5` |
| new | `services/core/src/identity/permissions.rs` | `3c1660fc6b616c2b95b90c18b3224b15f2f8797f91d26c9022397948a8178627` |
| new | `services/core/src/identity/admin_users/mod.rs` | `23f68971fdc04f96603f2389ef197673983288ff1932830c1eeae658b69d9c6a` |
| new | `deployment/postgres/grant-core-runtime.sql` | `b5e02a2191566b52cedaa1ffc7e07579aa6d2f5deb604ad0d875552c53dfd28d` |
| new | `scripts/migrate-legacy-financial.py` | `af599f91acb8bda9bc671cb71b21e7613cc5b30f3d03461347651ba355063ba6` |
| new | `scripts/financial_import.py` | `eca3fa4e99ea3b3e12e594c0c78c367601024f1ec363c34cad4cf7abc6410b3d` |
| new | `scripts/prepare_wallet_history_snapshot.py` | `a48e1266578263bdf9771385c8e9686325c0a92eb92829bf229e5f3bea324ca7` |

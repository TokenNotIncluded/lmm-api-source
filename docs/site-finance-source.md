# 全站财务来源：离线提取与导入

`scripts/legacy_site_statistics.py` 将冻结旧站材料转换为
`core.site-finance-source.v1`，供 Core 全站财务 reader 读取。它只向
`core_history.finance_users`、`finance_topups`、`finance_sources` 插入观测数据。
新鲜安装、升级、native payment VIEW 和 `core_meta.site_finance_upgrades`
由 Core 安装器负责；本工具没有 DDL、授权或账本写入口。

规则来源是父仓库固定提交 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` 的
`apps/api-go/model/usage_projection.go`、`referral_credit_rebase.go`、
`admin_site_statistics.go` 及 TopUp helpers。保留原项目、QuantumNous/new-api
及贡献者的 AGPL-3.0-or-later 归属。

## Python 合同

```python
normalized_usage(tables: dict[str, list[dict]]) -> dict[int, int]
classify_topup(row: dict, subscriptions: set[str]) -> FinanceTopup
build_plan(*, tables, identity_records, opening, source_table_counts) -> FinancePlan
load_source(opening_path: Path, supplement: dict | None = None) -> tuple
reconcile(plan: FinancePlan, snapshot: dict) -> dict
review_summary(plan: FinancePlan) -> dict
prepare(opening_path: Path, snapshot: dict, runtime_roles: list[str],
        supplement: dict | None = None) -> dict
build_apply_sql(plan: FinancePlan, snapshot: dict, runtime_roles: list[str]) -> str
apply_offline(material: dict, *, approval: dict, connection: dict, output: Path) -> dict
```

`build_plan` 是纯计算接口，输入已核对的原始完整表行、identity mapping 和
现有 financial opening material。实际文件流程使用 `prepare`：它先复用
`financial_import.py` 的私有路径、文件 hash、冻结 export、identity/opening
来源校验，再重建本工具的计划；不执行旧 opening/history importer。

`FinancePlan` 字段：

| 字段 | 含义 |
| --- | --- |
| `contract` | `core.site-finance-source.v1` |
| `source` | 下列原始 12 个输入字段；不包含数据库生成的 seal floor |
| `users` | 按 `user_id` 排序的所有保留用户，包括停用、软删除用户 |
| `topups` | 按 `source_order_id` 排序的所有 `status="success"` TopUp，包含 excluded |
| `normalization` | 固定旧源码 revision、算法名、相关表是否存在及 count/行摘要 |
| `opening_counters` | 原开账 counter payload，按 user_id 排序，11 个原字段全部保留 |
| `identity_import` | 原目标 identity import receipt |
| `plan_sha256` | 排除本字段后，排序紧凑 JSON 的 UTF-8 SHA-256 |

`source` 的字段为 `source_key`、`contract`、`source_archive_sha256`、
`mapping_sha256`、`tables_sha256`、`normalization_sha256`、`rows_sha256`、
`observed_at`、`user_count`、`topup_count`、`credits_per_usd=500000`、`complete=true`。
`mapping_sha256` 是原 financial opening mapping hash。
`observed_at` 等于原 opening 的冻结 dump 起始秒；保留原 observation 对
“精确财务快照时刻未知”的声明，不将它解释为订单支付或结算时间。
它也不是历史归档的时间上界，不能用 dumpstart+1 排除 Native 重叠。

`source_key = SHA256(JSON(source 去掉 source_key))`。该 JSON 使用
`sort_keys=True, ensure_ascii=False, separators=(",", ":"), allow_nan=False`。
`source_row_sha256`、`tables_sha256`、`normalization_sha256`、`rows_sha256`
沿用现有 financial planner 的 **带类型 digest**；不能改用普通 JSON hash。
`rows_sha256` 的输入是 `{"users": users, "topups": topups}`，此时叶行尚未添加
`source_key`。`tables_sha256` 绑定六张相关表各自的完整 count 和原始行数组 digest；
表不存在与已证明为空不同。空完整来源仍生成真实摘要和一条 complete source。

数据库目标封存值为 `finance_sources.native_intent_floor` 与 `native_request_ids`。
它们不属于原 source material，也不参与 SourceKey、rows hash 或 plan hash；调用者
不能提供或覆盖。INSERT 使用显式原 12 列加 `native_request_ids`，其中 request 集合
只来自事务内的 `ARRAY(SELECT id FROM core_charging.requests ORDER BY id COLLATE "C")`。
该列 NOT NULL 且无 DEFAULT。`native_intent_floor` 列省略，让 PostgreSQL 最后写
source seal 时执行 DEFAULT：
`COALESCE(pg_catalog.pg_sequence_last_value('core_payments.intents_id_seq'::regclass),0)`。
它在完整归档读取、其他客户端排空后记录目标已有 payment identity sequence 上界。
首次插入要精确回读等于该次现场上界，整个导入不得推进任何 sequence。

`finance_sql` 的 source 投影移除两个 target seal 字段，另由 `target_sql.source_seals`
保留每个 `{source_key, native_intent_floor, native_request_ids}`。快照同时输出
`native_intent_upper`、`native_intent_max_id` 与当前完整 `native_request_ids`；
reconcile 返回固化 `source_seals` 和当前目标观测。
首次及 NOOP 都检查 step=1、非 cycle、上界不低于 max(intents.id) 或既有 floor，
并与原始 sequence 的 last_value/is_called 状态相互核对。未调用 sequence 只有在
没有已存 Native ID 时才能解释为可信 0；rewind、手工高 ID、缺失 seal 证明均拒绝。
owner 必须显式拥有该 sequence 的 SELECT 权限；真实默认值必须与非 superuser
当前 owner 读取的原始函数值、last_value 和已有最大 ID 相互符合，不能悄悄用 NULL→0。
NOOP 核对并保留原 floor 和完整 request ID 集合，不随新 Native 数据重采。
target 已缺失封存 ID 时拒绝；ID 是 opaque 文本，仅以 C/UTF-8 字节序规范化集合，
不按字符串格式、前缀或时间猜其先后。目标发生变化后应重新只读快照和审核。

Core reader 的重叠判据是已入账 Native intent `id <= native_intent_floor`，不是创建时间。
因此 pre-seal 旧 ID 即使 created_at 晚于 dumpstart+1，后来入账仍须 Unavailable；
post-seal 新 ID 即使 created_at 与 dumpstart 同秒，也不因这个时间而被排除。
Native payment 的这两种 reader 响应由独立 owner 验证，writer 自测验证真实
seal/ID 关系与保留性。对 request 用量，reader 拒绝封存 ID 缺失，或封存 request
后来 settled 且 actor 是 finance_users 中的历史用户；不能再次叠加可能已有的历史用量。
完整 archive 不含、并由真实 Native creator audit 证明的用户仍可累计其 Native 用量，
旧历史基数为 0。post-seal 新 request 与 dumpstart 同秒授权也不受时间阈值阻挡。

`FinanceUser` 为 `user_id, account_id, source_row_sha256, raw_used_quota,
normalized_used_credits`；写入时添加 `source_key`。
用户 hash 是完整原 user 行的 opening 来源 hash，不是脱敏投影的新 hash。
原始 user/account/raw counter、archive、mapping、cutoff 和来源实体键
`legacy.users/<id>` 都必须与现有 opening counter 相同。

`FinanceTopup` 为 `source_order_id, source_row_sha256, classification, currency,
gross_amount_micros, refunded_amount_micros`；写入时添加 `source_key`。
`topup_count` 对应 reader 的完整 source topup 数量，包含 excluded。
`cash/virtual` 使用原 settled/refunded micros，其他三类金额和币种均为 null。
审核摘要中的总额为十进制字符串，按币种和 cash/virtual 分组，不跨币种求和。

## 原规则与材料完整性

归一化使用有理数，验证全部 `wallet_credit_rebases.plan`，包括不属于当前保留
用户的 audit scope。历史 baseline 按原 divisor、half-away-from-zero 或 toward-zero
舍入，再加迁移后原生增量。非 moderation 的已证明 violation fee reversal 同时
调整 raw 与 normalized baseline；moderation reversal 保留原不调整 usage 的语义。
缺失 audit、重复 scope、缺 baseline、反转来源改变或无法解释的 counter 减少均拒绝。
**catalog 证明 audit 表不存在**时使用原 native used counter，不按当前 K 或日志金额推算。
历史 token baseline 的缺项只影响原 Token()，不会额外阻断 User() 投影。

成功 TopUp 依次判定订阅镜像、内部排除项、来源/虚拟单位确认、非法金额和币种。
同 trade_no 的任意状态 subscription order 都排除；SQL NULL 不相等，空字符串会匹配。
LDC 仅按旧 ePay adapter 和不可变 snapshot 规则分类为 virtual。
不以 Money 浮点或 expected micros 补 settled 金额。必要数值缺失或类型非法时拒绝，
原规则可确认的未知来源保持 unconfirmed；pending/failed/NULL status 不属于成功充值。

必需原表为 `users`、`top_ups`、`subscription_orders`；catalog 中存在的
`wallet_credit_rebases`、`tokens`、`violation_fee_records` 必须有完整材料。
旧 financial export 可能未选择 `violation_fee_records`。其 baseline 明确为 0 时
可证明为空；非空时需要同一 archive 的完整补充 export。

补充 export 沿用 `migrate-legacy-financial.py:read_frozen_export` 的 JSONL：
一条 `kind=identity`、一条 `kind=schema`，以及 `kind=row, table, row`。
proof 必须含 `source_archive_sha256, source_export_sha256, metadata_hashes,
source_column_coverage_exact=true, source_row_baseline_exact=true,
selected_table_counts, source_instance`。本工具核对原 catalog 的全部列、原完整行数、
相同冻结 source instance；与原 opening plan 重叠的表还要逐行摘要一致。
补充材料是已授权离线导出交付物；本工具不会连接旧站补查或从 dump 启动数据库。

## 离线工作流

所有材料、连接配置、输出目录均使用现有 `~/Backups` 私有路径约束：目录 0700、
文件 0600，拒绝符号链接。输出目录预先创建且每次使用新目录；文件以 O_EXCL 写入并 fsync。
连接 JSON 沿用 `financial_import.py` 的本机 Unix socket owner/login_role/set_role 格式。
只读阶段也使用当前离线 owner；runtime 没有来源表写权限。

以下命令中的变量必须指向已审查的离线合成材料和独立输出目录：

```sh
python3 -B scripts/legacy_site_statistics.py --mode snapshot \
  --connection "$CONNECTION" --runtime-role "$RUNTIME_ROLE" --output "$SNAPSHOT_OUT"

python3 -B scripts/legacy_site_statistics.py --mode prepare \
  --opening-material "$OPENING_MATERIAL" --snapshot "$SNAPSHOT_OUT/target.private.json" \
  --runtime-role "$RUNTIME_ROLE" --output "$PLAN_OUT"
```

需要补充表材料时，在 prepare 命令中增加
`--site-export "$SITE_EXPORT" --site-export-proof "$SITE_EXPORT_PROOF"`。
原表、archive、map、metadata、proof、opening material 和工具源码 hash 在 apply 前后重验。
Core28 的 catalog 查询从本次冻结的 `core_schema.rs` 读取并绑定源码 hash，
不改用旧 opening importer 的 predecessor catalog 常量。

prepare 生成 `material.private.json`、`review.private.json`、`review.private.sql` 和
`approval-request.private.json`。最后一个文件中的 `decision=REVIEW_REQUIRED` 不能执行。
owner 审核完整计划/SQL 后，另存 approval，保持三个 hash/key 字段不变，显式设
`decision=approve-offline-site-finance`，然后才调用：

```sh
python3 -B scripts/legacy_site_statistics.py --mode apply \
  --material "$PLAN_OUT/material.private.json" --approval "$OWNER_APPROVAL" \
  --connection "$CONNECTION" --output "$APPLY_OUT"
```

snapshot 使用 REPEATABLE READ READ ONLY。apply 在取得共享离线 advisory fence 后
才建立 SERIALIZABLE 快照，启用 synchronous_commit，核对当前 owner、Unix socket、
离线连接限制、runtime 权限、instance、catalog、所有权与现有数据/sequence 摘要。
排空条件只用 `pg_stat_activity` 公开的 datid、非 NULL pid 与当前 pid 比较；不能筛选
普通 owner 看不到的 backend_type/state。NOLOGIN 不会断开已有连接，持有 sequence
cache 的旧客户端也必须退出。无需授予 pg_read_all_stats；背景 worker 宁可触发拒绝。
锁定 identity/opening、Native intents/requests 和目标来源表，事务内再次核对逐账户来源，插入所有 users/topups，
**最后**插入 complete source，检查 deferred FK、精确回读及其他表/权限未变后提交。
新旧升级回执都在保护摘要内。唯一键或任一字段冲突拒绝，完全相同来源重放返回 NOOP。

完整历史来源只有一个全局 namespace，不合并多个 archive。`target_sql` 显式输出
`source_namespace.source_count/source_keys/archive_sha256s/user_rows/topup_rows`，
全部是无候选 key 过滤的全表观测。prepare/reconcile 先拒绝不同 source_key、多来源，
或无完成标记的叶行；apply 在事务内再次显式检查 `site_finance_source_key_conflict`。
Core 的 `finance_sources_one_archive` 唯一索引作为数据库约束，不能替代该前置核对。

持久记录 attempt 和实际 SQL 后仅发起一次 psql；没有自动重试。
非零退出、断连、超时、无法确认回执、提交后输入变化或结果落盘失败都保留 UNKNOWN，
不能由客户端推断已回滚。使用新的只读 snapshot + prepare 核对准确 source_key 和完整行，
确认旧连接结束后由 owner 决定下一步。stdout 只有状态和摘要；原始来源行仅留私有材料。

## 当前检查边界

```sh
python3 -B scripts/test_legacy_site_statistics.py -v
```

测试覆盖真实本地合成文件解析与 hash、归一化/反转、充值分类、完整性、三方冲突、
空完整来源和执行器持久记录/UNKNOWN 分支。执行器中的数据库 transport 是 mock，
SQL 检查是生成合同检查，单独不能证明 PostgreSQL 执行、事务回滚、owner 权限或
deferred FK 行为；这些由下面的真实自测覆盖。Core reader 集成、升级及生产切换
仍需各自的独立验收。

## 作者真实 PostgreSQL / 完整 CLI 自测

`tests/site_statistics/importer_selftest.py` 与 `importer_fixture.py` 使用自己的合成
旧库，真实 pg_dump → pg_restore → 只读 export，再实际执行 snapshot、prepare、
owner approval、apply 和 reconcile CLI。目标由冻结 Native 的 fresh28 init-db 创建。
它不编译 Core、不消费独立 reader oracle，也不执行 27→28 升级。

```sh
python3 -B tests/site_statistics/importer_selftest.py \
  --candidate "$FINAL_R6_OR_LATER" --root "$NEW_SHORT_PRIVATE_BACKUP_ROOT" --scope seal --execute
```

启动前先运行 space-first；candidate 必须具有 passed result、binaries.json 与全部
冻结输入，工具自行核验 hash 和实际 schema-manifest；已 superseded 的 R5 不启动。
新 root 必须是 `~/Backups`
下不存在的短目录。执行使用共享 `/tmp/lmm-delegate/heavy-build.lock`，MemAvailable
至少 2GiB，实际用户 unit 限额 MemoryMax=2G、MemoryHigh=1536M、MemorySwapMax=4G、
CPUQuota=100%。没有数据库或安装器 SQL fallback。

`--scope seal` 是当前默认范围：必要的 fresh28/完整 archive 初始化，以及新 floor
和 opaque request 集合捕获、cached allocator 排空、post-seal 新 ID、NOOP 不重采和
step/cycle/rewind/手工高 ID 拒绝。
fixture 明确设定 pre-seal created_at=dumpstart+10、post-seal created_at=dumpstart；
期望值来自 ID 与 seal 的先后关系，不从当前 reader 返回推导。
request 用例还通过冻结 Native bootstrap 创建真实 audit 的归档外用户，使用符合
schema 的 SERIALIZABLE request 状态夹具，实际调用 Native HTTP：旧历史 pending
后来 settled 必须 503；同秒 post-seal 请求和有真实 audit 的 Native 用户用量必须
按手算整数累计。此处不重新验收 charging producer 或上游供应商执行。

`--scope base` 单独运行原 10 组：fresh28/完整 archive 初始化，以及完整分类和归一化、精确 NOOP、prepare/apply 的不同源拒绝、完成标记前注入
故障后的整笔回滚、真实 psql 已提交却丢回执时 UNKNOWN、原始文件改变时提交前拒绝，
以及 runtime 写权限拒绝。独立于 importer 的快照对比所有保护表、序列实际状态、
catalog 和权限。故障函数只由测试 owner 在一次性 fixture 内创建；runtime 不执行 DDL。
成功后停止并删除该 PG 复制体和 Native 复制体，保留日志、hash、单一源 archive 和
冻结 Python 依赖。此结果属于开发者自测，独立 reader 验收由另一 owner 完成。
`--scope full` 一次运行 base 与 seal 的并集，共 14 组；两个范围共有初始化的 2 组。
R3 已通过的 10 组记录保留原候选范围，不能算成 R6 seal 或 reader 验收。

# 钱包历史只读导入

`GET /api/user/topup/self` 读取安装 owner 导入的 `wallet-topup-history.v1` 不可变快照。
导入只追加 `core_migration.import_runs` 回执，不重放充值、不改变余额、账本或支付意图。
当前工作项见[项目进度](progress.md)，历史交接与验收记录见[归档](archive.md)。

## 来源与读取边界

| 数据来源 | 含义 |
| --- | --- |
| 旧库 `top_ups` | 本指南的充值订单快照 |
| `core_history.log_records` 的 `log_type=1` | 独立的历史日志，不能按条数与充值订单相减来判断缺失 |
| `core_payments.intents` | 后续原生支付意图，不包含在旧订单快照中 |

接口只接受有效 Web 会话，按目标用户与其个人账户同时限定范围。已映射但没有订单
的用户可得到有依据的空页；映射未覆盖的用户返回 `503 TOPUP_HISTORY_SCOPE_UNAVAILABLE`。
未安装或无法确认唯一快照时返回 `503 TOPUP_HISTORY_SOURCE_UNAVAILABLE`。

Core 启动装配已调用 `WalletTopupHistory::installed`：仅存在一份该 scope 的已安装
回执时启用读取。无需手工挂路由或新增 migration ID 配置，请求参数也不能选择快照。
实现见[主程序](../services/core/src/main.rs)、[快照读取](../services/core/src/wallet_topup_snapshot.rs)
和[HTTP 接口](../services/core/src/wallet_history_read_http.rs)。

## 准备离线快照

导出器和准备器目前绑定 `lmm_api.lmm_prod_20260802.top_ups` 来源合同。只对已审核
旧备份的离线恢复库运行[导出 SQL](../scripts/export_wallet_history_snapshot.sql)；
其他来源须先审核适配合同，不能改来源名称绕过校验。连接使用私密配置，不把凭据写进命令。
下例的 psql service `wallet_history_source` 指向该离线恢复库。

```sh
umask 077
psql --no-psqlrc --quiet --tuples-only --no-align --set ON_ERROR_STOP=1 \
  --dbname='service=wallet_history_source' \
  --file scripts/export_wallet_history_snapshot.sql > /secure/path/topups.json
```

身份映射来自已审核的身份迁移材料，须覆盖导出中的全部源用户，包括零订单用户。
JSON 顶层为数组，每项只含以下字段；目标用户及个人账户必须已经存在：

```json
[{"source_user_id": 7, "target_user_id": 71, "target_personal_account_id": 701}]
```

将 `WALLET_HISTORY_EXPECTED_ROWS` 设为独立核对的源订单条数，再运行准备器。
输出目录必须尚不存在，生成的计划绑定备份、导出、映射及每条源记录的摘要。

```sh
python3 scripts/prepare_wallet_history_snapshot.py \
  --source-export /secure/path/topups.json \
  --identity-map /secure/path/identity-map.json \
  --source-archive /secure/path/lmm_api.dump \
  --expected-rows "${WALLET_HISTORY_EXPECTED_ROWS:?set independently verified source count}" \
  --output /secure/path/wallet-topup-history-import
```

审核 `plan.json` 和 `manifest.json` 后才能应用。条数不符、重复订单、未知状态、
非法金额或缺少身份映射会阻止生成可用导入；保留 `blocked.json` 说明原因。
金额保留原始十进制文本，未知币种单独汇总，不按现价推算 USD。
准备器本身不连接数据库，合同见[生成脚本](../scripts/prepare_wallet_history_snapshot.py)。

## Owner 应用与核对

使用拥有 `core_migration.import_runs` 的离线安装 owner。下例的
`wallet_history_target_owner` service 指向已审核目标；先运行生成的原事务，再只读核对：

```sh
psql --no-psqlrc --set ON_ERROR_STOP=1 \
  --dbname='service=wallet_history_target_owner' \
  --file /secure/path/wallet-topup-history-import/apply.sql
psql --no-psqlrc --quiet --tuples-only --no-align --set ON_ERROR_STOP=1 \
  --dbname='service=wallet_history_target_owner' \
  --file /secure/path/wallet-topup-history-import/verify.sql
```

事务获取安装锁、重验目标身份归属，并读回完整 JSONB 回执。相同准备材料可幂等
重放；同一 migration ID 更换备份、数据或映射会被拒绝，另一份来源快照也不能混入。
核对源条数、导入条数、缺失条数、状态与币种汇总以及 `exact_manifest_match`；
历史日志和原生支付意图只分别报告，不要求与订单条数相等。

在既有最小 runtime 权限之外，读取回执需要以下授权；由 owner 应用并运行现有
数据库权限检查，服务启动不代为导入或授权：

```sql
GRANT USAGE ON SCHEMA core_migration TO lmm_core_runtime;
GRANT SELECT (migration_id, manifest)
  ON core_migration.import_runs TO lmm_core_runtime;
```

## 返回合同与检查入口

查询支持 `p/page_size`、`keyword`、`sort_by/sort_order`；默认第 1 页、每页 20 条，
每页最多 100。返回保留原订单 ID、交易号、状态和时间，`user_id` 使用已审核映射。
`money` 保持兼容数值，`money_text` 保存原始精确文本，覆盖信息说明来源与导入范围。
`/api/user/topup/info`、管理员充值列表和原生支付订单属于其他接口。

变更读取或导入逻辑时，相关回归入口为
[快照准备测试](../scripts/test_prepare_wallet_history_snapshot.py)和
[HTTP／隔离 PostgreSQL 测试](../services/core/tests/wallet_history_read_http.rs)。
它们覆盖幂等导入、只读权限、账户隔离、精确金额、零订单与未覆盖用户等边界；
此处不记录某一轮通过数量，也不据此声明真实历史已经导入。

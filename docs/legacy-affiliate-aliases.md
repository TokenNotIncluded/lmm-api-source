# 旧邀请码保留与 Core86 导入

[Core 与 Extensions 架构](core-migration.md#旧数据迁移与证据) · [PostgreSQL 权限](../deployment/postgres/README.md)

Core86 为旧站四位邀请码增加只读别名，保留原大小写及所属用户。原生 32 位邀请码继续有效；同一用户同时有两种代码时，读取接口优先返回旧码。注册归属仍由 Core 验证邀请人及其个人账户均有效。导入保留停用、删除用户的历史归属，运行时不因此恢复其邀请资格，也不发放奖励。

实现以 [alias schema](../services/core/schema/legacy_affiliate_aliases.sql)、[邀请归属代码](../services/core/src/affiliate.rs)和[导入模块](../scripts/current_affiliate_aliases.py)为准。NULL 代码保留在完整源清单中，不生成替代码；四位代码只接受 ASCII 字母和数字，不裁剪、不转小写。

## 结构升级

已有原生库通过离线过程 `core-legacy-affiliate-aliases-v1` 从精确 Core85 升到 Core86。请求沿用 Core 的目标实例、owner、前置 catalog、原行和序列绑定；匹配入口见 [升级注册表](../services/core/src/core_schema/upgrade/registry.rs)。普通启动不会执行升级。

| 合同 | Manifest SHA256 | Catalog SHA256 |
| --- | --- | --- |
| Core85 前置 | `2c4563c35a29fe5c0d579c5d0ea41e9300a3027343fbae641ad098d5524e8d80` | `1890e9f888a13b535dd653bffe5771745c6e4e44421211ae6723fcffedd64462` |
| Core86 目标 | `ddd230cfd5c336d0f8f9245932b840db8d88b6c422308360bf6a0b06473a899b` | `1936d5baa2a16385260975aa7b995609f8c9636759fe36ba3a51c7e7d3396828` |

结构升级只建立表、约束和升级回执，不导入邀请码或移动资金。它只向原有注册 reader 追加 `SELECT(code,user_id)`，不启用原先关闭的业务。新安装时，这两列读取随 `enable_web_identity=true` 授权。完整源行、导入材料及 owner 回执不向 runtime 开放；Core 和 Go runtime 均无别名写权限。

## 源预览及冻结依赖

`current_affiliate_aliases.py source-preview` 读取已经导出的完整源凭据和证明，生成私有预览及不含邀请码的数量摘要。它不连接数据库，不生成源停写证明、身份提交回执或批准文件。

本脚本复用已审阅的迁移 helper。发布源码没有携带实际站点材料，依赖来自当前仓库及显式提供的 `--support` 目录：

- 冻结支持目录：`current_key_lineage.py`、`migrate_current_identity.py`、`final81_target_guard.py`、`migrate_current_runtime_keys.py`。
- 当前仓库的 `scripts/`：`legacy_routing_migration.py`、`migrate-legacy-financial.py`、`financial_import.py`、`migrate-legacy-snapshot.py`、`legacy_credential_import.py`、`legacy_api_key_migration.py`、`legacy_key_quota_opening.py`。

每个文件的精确 SHA256 保存在导入模块的 `FROZEN` 常量中，加载时逐个核对。这里复用其源验证、身份回执读取和状态观察逻辑；不改写旧 helper 的 Core78/Core81 目标，也不把旧验收结果改标为 Core86。冻结目录和源凭据应来自同一次审阅的迁移包。

在仓库根目录设置下列私有输入路径后生成新文件；输出路径必须尚不存在：

```sh
python3 -B scripts/current_affiliate_aliases.py source-preview \
  --support "$LMM_MIGRATION_SUPPORT" \
  --source-receipt "$LMM_SOURCE_RECEIPT" \
  --source-proof "$LMM_SOURCE_PROOF" \
  --output "$LMM_ALIAS_PREVIEW"
```

命令写入完整私有材料，终端只返回路径、哈希和汇总。源凭据、原始用户行、密码信息、代码、身份映射及连接信息均不进 Git。

## 导入与恢复条件

导入模块提供 `prepare_owner`、`transaction_sql` 和 `resolve_sql`，供已审阅的离线 owner 驱动调用；目前没有通用生产 `apply` 命令。准备材料需要同时具备以下事实：

1. 完整源导出及其证明逐字节校验通过，所有用户均有源行摘要，代码及源用户无重复。
2. c343 身份迁移已经真实提交，其 `core_migration.import_runs`、完整源用户映射和目标身份读回一致。预览中的拟分配 ID 不能代替提交回执。
3. 源停写证据绑定同一导出、捕获时间和运行实例；没有源停写证明时只允许预览。
4. 目标实时 catalog、component 清单与上述 Core86 合同一致；使用独立 NOLOGIN owner 和 NOINHERIT installer，经 Unix socket 离线连接，其他连接、预备事务和 runtime 写入口已隔离。
5. 审阅文件精确绑定材料摘要、实际管理员、源停写证明和目标实例。结构升级成功不提供这份导入批准。

实际 SQL 在一个 SERIALIZABLE 事务中取得共同迁移锁，比较准备时的完整行与序列状态，再写入完整导入回执和全部非 NULL 别名。延迟约束在提交前核验完整批次；既有表与序列必须保持不变，除别名表及其导入回执外不写其他记录。

同一材料的重放只接受精确原回执和全部别名。发现已有但不匹配的导入即拒绝。提交结果不明时先使用原请求的 `resolve_sql` 只读确认目标实例、原执行连接、回执及全部别名；不自动再发导入事务，也不把连接断开判为失败或成功。

## 隔离验证

纯输入和准备条件检查：

```sh
PYTHONPATH="$LMM_MIGRATION_SUPPORT" \
  python3 -B scripts/test_current_affiliate_aliases.py
```

真实 PostgreSQL 机制检查使用合成账户和合成代码。它会建立仅 Unix socket 可访问的独立实例、执行真实 c343 身份事务、安装匹配的85/86结构，并在结束时停止该实例：

```sh
python3 -B scripts/check_legacy_affiliate_aliases_pg.py \
  --source "$LMM_SOURCE_CHECKOUT" \
  --capture85 "$LMM_FINAL85_CAPTURE" \
  --support "$LMM_MIGRATION_SUPPORT"
```

运行环境需提供 PostgreSQL 的 `initdb`、`pg_ctl`、`psql` 和 Python。`--source` 默认当前脚本所在仓库，显式目录应包含固定 Git 历史 `fa57cd2049ccd9138cdc0b4104db36b5b6c53ae5`，驱动由该历史读取 Core78 授权脚本。`--capture85` 需含匹配的 `manifest85.json` 及 `source-bindings.json`，后者的 SHA256 为 `7f012aa45d68432560d0defad1580f5f56934c8f6d75432e08e328eb36b0bf51`。

该检查的冻结支持包还需包含 `check_final81_gift_pg.py`、`test_current_identity_pg.py`、`test_current_key_production.py`、`test_migrate_current_runtime_keys_pg.py`、`seed.sql` 及这些文件引用的依赖。应沿用冻结包清单，不能从不同版本零散拼接。检查结果保留驱动、模块、schema 和 capture 的字节绑定。

此检查覆盖完整批次、并发 owner 隔离、状态变化拒绝、提交前回滚、提交确认丢失后的只读恢复、精确重放及 runtime 两列权限。实际数据导入、编译后二进制的 owner 升级入口和上线业务验收各自保留独立证据，当前状态见[项目进度](progress.md)。

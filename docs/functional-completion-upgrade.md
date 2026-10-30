# 功能补齐批次的离线数据库升级

本页对应 `core-functional-completion-v1`，将固定的 32 组件数据库升级为
41 组件数据库。该程序只接受 PostgreSQL 18 和下列准确结构；不是通用 SQL
迁移器，也不会在服务启动时自动执行。真实历史数据导入、第三方支付验收和
生产切换仍须按各自的验收流程执行。

## 固定输入与输出

| 合同 | SHA-256 |
| --- | --- |
| 原 32 组件 manifest | `9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15` |
| 原 PostgreSQL 18 catalog | `abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce` |
| 新 41 组件 manifest | `12c971ce84ca71eb5605a7fe284535216a5777858d5aaf766c3876655bf8aee6` |
| 新 PostgreSQL 18 catalog | `05d1aa6913236fc12cc099640ead1ac2d88a1bde073923ce8353907cfc46c8bb` |

新 catalog 来自隔离 PostgreSQL 18.6 上的真实 `init-db` 安装。编译进程序的
目标位于 `services/core/src/core_schema/upgrade/completion_target.json`。
`schema-manifest` 的完整原始 JSON（包括末尾换行）用于计算 manifest 哈希；
catalog 则按 manifest 中的查询和长度前缀算法计算，不能对 SQL 文本或格式化后的
JSON 另算一个哈希来替代。

新增组件包括渠道控制、加密密钥保管、Passkey、账户注销、订阅、营销、通知、
第三方账号绑定和升级收据。历史组件的 SQL 字节保持不变；需要变更的现有约束或
函数由新组件显式执行。渠道密钥还要求配置独立的 32 字节主密钥文件，数据库升级
不会生成、恢复或向客户端返回该密钥。

## 执行前的具体材料

先冻结待升级站点的源码、二进制、数据库备份及恢复证据。收集数据库实际名称、OID、
PostgreSQL system identifier、server version 和四个角色名，生成一份受审批的请求。
安装器登录角色与 NOLOGIN 所有者分离；运行角色和扩展角色也必须独立。
请求中的角色、数据库标识和结构摘要都由程序再次核验。

停止该数据库的业务进程并阻止新连接。程序会检查其他连接和角色边界，取得排他维护
锁，并在 SERIALIZABLE 事务中锁住原有表；检查失败就拒绝升级，不自动终止连接、
改角色或清空数据库。

请求字段如下。示例中的标识是说明用值，必须替换为审批材料中读回的实际值。
不得将安装器口令写入请求或命令行；使用私有 TOML 与其引用的私有数据库 URL 文件。

```json
{
  "version": 1,
  "procedure": "core-functional-completion-v1",
  "upgrade_id": "approved-site-upgrade-identifier",
  "database": {
    "name": "replace_from_preflight",
    "oid": 1,
    "system_identifier": "1",
    "server_version_num": 180006
  },
  "roles": {
    "installer": "replace_installer",
    "owner": "replace_owner",
    "runtime": "replace_runtime",
    "extensions": "replace_extensions"
  },
  "predecessor_manifest_sha256": "9c9726a7fa204a150d9fe18139d4580d23feb4cec40145fcb829338331a4cd15",
  "predecessor_catalog_sha256": "abe3979b376da869d4a05b32dda8c3ad33197497db7af16970d8c9b1153bc6ce",
  "successor_manifest_sha256": "12c971ce84ca71eb5605a7fe284535216a5777858d5aaf766c3876655bf8aee6"
}
```

审批完成后，使用与上述目标相符的冻结安装器运行：

```bash
lmm-core --config /absolute/private/installer.toml upgrade-db \
  < /absolute/private/approved-upgrade.json \
  > /absolute/private/upgrade-result.json
```

## 保全、失败与恢复

应用新增 DDL 前后，程序分别对原有数据行、序列状态、对象所有者和既有权限进行
比较。新增对象不被误算成原权限变动；替换的既有函数仍须保留其权限。
任何差异、错误的原库指纹或不匹配的新库指纹都会使事务回滚。

成功结果为 `committed`。升级 ID、原始请求及摘要、保全摘要和目标 catalog
写入 `core_meta.completion_upgrades` 的不可变收据。该表只属于安装所有者，
运行角色不能读取或修改它；误授读取权也会使恢复检查失败。

若连接中断导致提交结果未知，保持站点关闭，保留同一份请求和同一个升级 ID。
用相同冻结安装器和原请求重新查询执行结果：准确的已提交收据返回
`already_committed`；不同 ID、不同请求或发生结构偏移均拒绝。不得用新 ID、
重新初始化、直接补写组件摘要或跳过指纹来处理未知结果。

事务成功后，按部署启用的实际能力执行本版本
`deployment/postgres/grant-core-runtime.sql` 中的最小授权，并读回检查。
账户绑定的撤销状态只读权限是基础身份能力的一部分；Passkey 与绑定写权限随
Web 身份能力启用；订阅、营销与通知随 console 能力启用；渠道操作依赖模型能力。
最后用独立运行角色执行 `check-db`，完成受影响业务验收后再按生产切换流程开放站点。

## 验证入口

`services/core/tests/completion_upgrade.rs` 使用真实分离角色和冻结 32 组件前身，
验证升级、用户和账本保全、准确重放、错误 ID 以及收据权限。
`services/core/tests/common/completion_predecessor.rs` 仅构造该测试前身，
不是生产导入工具。各功能的正常交易和最小权限测试与升级测试分别执行，
新库安装成功本身不代表全部业务已验收。

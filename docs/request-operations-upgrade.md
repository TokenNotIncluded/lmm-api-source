# 请求与运维状态：固定 Core 29 → 31 离线升级

本轮在既有 29 个组件后新增 `request-limits` 和 `operations`。正常启动只检查数据库，
不执行 DDL。全新数据库使用当前安装器的 `init-db`；已有数据库必须匹配下述前置合同，
通过显式 `upgrade-db` 升级。源码为
[`request_operations.rs`](../services/core/src/core_schema/upgrade/request_operations.rs)。

## 固定合同

| 项目 | 值 |
| --- | --- |
| procedure | `core-request-operations-v1` |
| 原组件数 / 新组件数 | 29 / 31 |
| 前置 manifest stdout SHA-256 | `48578c69632d80d7d4e41a5fb36c67f365875e8bb67e0695f62e1d688177f530` |
| 前置 catalog SHA-256 | `cb9349ca60bafbd49204dc237a88eec5ca10ac4654e654e7c816ea59673bc21e` |
| 新 manifest | 当前候选 `lmm-core schema-manifest` 的完整 stdout（含结尾换行）SHA-256 |
| PostgreSQL | 18，实际完整版本同时绑定在请求中 |

前置 catalog 使用已冻结的
[`payment_gateways_target.json`](../services/core/src/core_schema/upgrade/payment_gateways_target.json)。
安装器验证旧 29 个组件的 SQL 哈希均未改变，只执行两个新增组件的 SQL。
新 catalog 去除这两个命名空间，以及新外键在既有 `users` 表上增加的四条固定 RI
触发器后，必须逐条等于冻结的旧 catalog。四条触发器逐字段固定并按出现次数扣除，
不能用相同约束名跳过其他旧定义。

请求沿用 `UpgradeRequest` v1：`version`、`procedure`、唯一 `upgrade_id`、
`database`（实际名称、OID、system_identifier、server_version_num）、
`roles`（installer、owner、runtime、extensions）、前置 manifest/catalog 哈希与
新 manifest 哈希。数据库身份和角色必须取自目标离线环境；不能使用测试值或猜测值。

调用方式保持为：

```sh
lmm-core --config /absolute/installer.toml upgrade-db < /private/original-request.json
```

请求只从标准输入读取，配置中使用安装器凭据。不要把数据库密码或应用密钥放入命令行。
`upgrade-db` 不是通用迁移器：更早版本需要保留的对应安装器和既定升级链。
28 → 29 仍由[支付网关固定升级](payment-gateways-upgrade.md)的 29 组件安装器执行。

## 事务、权限和重试

沿用已有离线升级的数据库围栏、安装器 `LOGIN/NOINHERIT` 与所有者 `NOLOGIN` 约束、
应用连接关闭检查、串行化事务和表锁。执行前后比较旧业务行、序列状态与旧对象权限。
新增权限只覆盖请求限速的受控函数、配置读取，以及运维任务和心跳所需操作。

运行时不能直接读写限速内部计数表、修改清理任务进度，或读取升级凭据；扩展服务
不获得两个新命名空间的业务权限。历史日志删除仅由固定的批处理函数执行，
不会给运行时授予历史日志表的直接 `DELETE` 权限。

升级凭据位于 `core_operations.upgrade_receipts`，绑定完整原请求、保留数据与权限哈希、
新 catalog 哈希，并禁止修改、删除和截断。相同原请求重放返回 `already_committed`。
更换 upgrade ID、修改绑定或丢失原凭据均拒绝。提交确认丢失时应保留原请求，
不能通过创建新 ID 假定旧事务已经回滚。

## 开发自测与部署边界

[`request_operations_upgrade.rs`](../services/core/tests/request_operations_upgrade.rs)
使用隔离 PostgreSQL、随机合成角色与 SQLx 专用数据库，重建并逐字验证固定的 29 组件
前置 catalog，执行升级和原请求重放，检查旧用户数据、31 个组件凭据及新权限边界。
另有[纯单元测试](../services/core/src/core_schema/upgrade/request_operations/tests.rs)
覆盖错误数据库/角色/哈希绑定与非预期 catalog 定义。

PG 测试命令为 `cargo test --locked --offline --test request_operations_upgrade`，必须设置指向隔离
PostgreSQL 的 `DATABASE_URL`。实际执行结果见[本轮交付记录](request-operations-completion.md)。
这些开发自测不构成生产升级或独立验收记录。此轮提交没有执行生产迁移、部署或站点开放。

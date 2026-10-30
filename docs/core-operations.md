# Core 日志清理与实例心跳

这两个功能由 Rust Core 和 PostgreSQL 持有状态。源码入口为
[`operations.rs`](../services/core/src/operations.rs)，数据库组件为
[`operations.sql`](../services/core/schema/operations.sql)。旧数据库先完成
[29 → 31 离线升级](request-operations-upgrade.md)。

## 可续跑的历史日志清理

Web 设置的日志清理使用以下真实接口；必须以当前超级管理员会话认证。

| 接口 | 行为 |
| --- | --- |
| `POST /api/system-task/log-cleanup?target_timestamp=...` | 创建任务，必须携带单个 `Idempotency-Key` |
| `GET /api/system-task/current?type=log_cleanup` | 读取最近任务，包含已完成任务 |
| `GET /api/system-task/list?limit=20` | 最近任务列表，最多 100 条 |
| `GET /api/system-task/{task_id}` | 读取指定任务与进度 |

时间戳使用 Unix 秒，必须为过去时间。任务只删除 `core_history.log_records`
中早于该时间、且属于创建任务时已有导入来源的历史日志。后续导入的数据不进入旧任务。
导入来源凭据、原生计费账目、模型执行与恢复证据均保留，页面确认文字明确此范围。

任务 ID 由管理员身份和幂等键派生。相同键、相同时间返回同一任务；相同键改变时间
返回冲突。同一时间只允许一个未完成任务，避免多个清理任务交错计算进度。
Web 在失败重试时保留同一个幂等键，收到确认后才清除本地尝试状态。

后台每次最多删除 500 条，删除行和 `processed` 进度在一个数据库事务提交。
进程在提交前中断时，该批次与进度均回滚；提交后中断时，下个进程从已提交进度继续。
多个 Core 使用 `FOR UPDATE SKIP LOCKED` 领取批次，不会重复计算同一批次。
单批使用 5 秒 statement timeout；存储故障不会把任务伪标为完成，后续循环继续尝试。

状态为 `pending`、`running`、`succeeded`。响应包含 `total`、`processed`、
`remaining`、`progress` 和完成后的 `deleted_count`。该功能不提供暂停、取消或
任意表删除接口；管理员不能通过请求参数改变清理表或批处理 SQL。

## 多实例心跳

`GET /core/v1/admin/instances` 接受当前管理员或超级管理员会话。返回最近 200 个
Core 启动实例；超出时 `truncated=true`，页面显示截断提示。

每个进程启动生成独立 `boot_id`，同名实例和重启进程不会覆盖彼此。可用已有
`LMM_CORE_INSTANCE` 配置提供展示名称；未配置时生成带启动 ID 前缀的名称。
这只是展示名称，不承担身份或主从选举功能。

心跳正常每 10 秒写入，过期判断使用 PostgreSQL 时钟：超过 45 秒或状态为
`stopped` 即 `online=false`。每小时移除超过 7 天的旧心跳。进程排空期间仍更新
`draining` 状态；请求退出后写入 `stopped`。意外退出依靠 45 秒超时显示为过期。

每条记录提供版本、启动/心跳时间、生命周期阶段、进行中的请求数量，以及模型/RPC
就绪状态。Web 每 10 秒刷新，过期实例的模型就绪状态显示为不可用。心跳没有远端
CPU、内存或主从身份采样；本机资源页继续使用独立的原生资源接口。

两个接口组均返回 `private, no-store` 和 `Vary: Authorization`，拒绝普通用户访问。
运行时权限检查验证新增表与函数的最小授权；升级凭据不对运行时开放。

## 自测入口

[`tests/operations.rs`](../services/core/tests/operations.rs) 使用真实 PostgreSQL、
受限运行时角色和 HTTP Router，验证幂等创建、缺少请求键、批次回滚/重启、并发执行、
后导入数据保留、账目不变、心跳过期、同名实例与权限拒绝。
Web 自测位于
[`system-info/api.test.tsx`](../apps/web/src/features/system-info/api.test.tsx)。
实际执行结果见[本轮交付记录](request-operations-completion.md)。

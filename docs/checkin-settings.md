# 签到在线设置

## 范围与机制

LMM API 的签到设置使用 `GET /api/option/checkin` 和
`PUT /api/option/checkin`，由 Extensions 接收，并向 Core 重新验证当前浏览器会话。
只有平台所有者（Core platform level 6）可以读写。旧 `/api/option/` 的只读状态
不再控制这个页面。

设置保存在签到专用 PostgreSQL 数据库的 `lmm_checkins.policy_revisions` 和
`active_policy` 中。保存追加一个版本，并在同一事务内更新生效指针；读取、签到状态
和领取流程使用同一份持久化策略。每次保存需要旧版本号和操作键：旧版本返回冲突，
相同操作与相同内容返回原结果，相同操作键配不同内容会拒绝。

金额通过十进制整数字符串传输，范围为 0 至 9223372036854775807，最低值不得高于
最高值。在线保存把奖励倍率统一设为 1，表单没有等级倍率。历史版本与历史领取记录保留。

资金仍由 Core 独立授权。保存时若能读取当前账户的 Core 上限，启用的策略不得超过该
上限；领取时 Core 仍逐账户执行资金上限、会话、人机验证与每日幂等约束。保存设置
不触发奖励发放，也不增加 Extensions 对 Core 账本的数据库权限。

实现来源：

- [HTTP 会话与权限检查](../services/extensions/internal/webcompat/checkins_settings.go)
- [配置读写与 Core 上限查询](../services/extensions/internal/modules/checkins/settings.go)
- [限定写入范围的数据库函数](../services/extensions/internal/modules/checkins/settings.sql)
- [实际签到策略读取与领取](../services/extensions/internal/modules/checkins/service.go)
- [Core 资金边界](../services/core/src/checkin_ledger.rs)

## 保存能力与领取状态

页面分别显示以下状态，避免把“策略已保存”解释为“奖励已可领取”。

| 返回字段 | 含义 |
| --- | --- |
| `writable` | 专用数据库已安装限定函数，且当前运行角色有执行权。 |
| `module_enabled` | Extensions 当前是否允许 checkins 模块的业务路由。停用模块后仍可管理已保存策略。 |
| `funding_available` | 当前会话成功读取了 Core 的签到资金权限。 |
| `funding_limit` | 当前账户的 Core 每日上限；不可读取时为 null，并非所有账户的共同上限。 |

Core 资金接口暂不可用时，只要签到服务和专用数据库已装配，保存入口仍可工作。
模块未装配或专用数据库不可用时返回 503。缺少在线设置升级时可以读取策略，但
`writable=false`。领取另外要求模块启用及人机验证可用；本接口不创建挑战来探测就绪，
所以不返回未经验证的“可以领取”承诺。

服务端失败时 Web 保留编辑内容。写入结果未知时，用户再次保存原内容会复用同一操作键；
发生版本冲突时保留草稿，并提供明确的“放弃修改并重新加载”。

## 已有数据库的安装前置

升级仅适用于已存在、契约版本为 1 的签到专用数据库。不修改 Core schema，不安装资金
配置，不改变已生效的签到策略。新建数据库的正式 fresh-install 流程已经包含该函数，
无需重复执行此升级。

生产操作前需要确认：

1. 已备份目标数据库，并核对数据库、owner、runtime 三个名字及对应的部署版本。
2. 只有独立的离线操作者持有 owner 连接文件；文件遵守现有私密文件规则。owner 必须
   为 NOLOGIN，操作者与相关角色均不能是超级用户或持有创建角色等高权限。
3. 停止访问该专用数据库的服务和监控连接，将 runtime 角色隔离为 NOLOGIN，并关闭
   它已有的连接。升级拒绝其他会话、预备事务、可连接的其他普通登录角色以及 runtime
   的角色成员关系。此隔离过程由部署负责人安排，不由升级命令暗中执行。
4. 保留部署前角色与服务状态，用于成功或失败后的恢复。正常服务启动仍只接收 runtime
   凭据，不能持有 owner 凭据。

命令由 [CLI](../services/extensions/internal/freshinstall/checkin_settings_upgrade.go)
调用[事务升级实现](../services/extensions/internal/modules/checkins/settings_upgrade.go)。
安装内容来自编译进二进制的 `settings.sql`，新增一个 SECURITY DEFINER 函数和升级回执。
runtime 仅获得这个函数的 EXECUTE 权限，不能直接改写策略版本表。

下面是参数示例，名字需要替换为已经核对的目标，不能照抄到未知数据库：

```sh
./extensions upgrade-checkin-settings \
  --owner-url-file /run/secrets/checkins-owner-url \
  --database lmm_checkins_live \
  --owner-role lmm_checkins_owner \
  --runtime-role lmm_checkins_runtime \
  --operation-id checkin-settings-20261117
```

成功后核对输出中的数据库、操作 ID 和 settings_version=1，再按部署记录恢复 runtime
登录能力与服务。使用真实所有者会话读回 `/api/option/checkin`，检查版本、金额和
`writable`；这是读取验收，不需要发起真实领取。

## 结果未知与回滚

若连接在提交阶段中断，应保持隔离状态，用原操作者、原目标和原操作 ID 重跑命令并添加
`--recover`。恢复只核对原升级回执，不重新安装或切换策略。不同操作 ID、操作者或数据库
不算同一个已知结果。确认结果前不要创建第二个升级意图。

撤回在线管理能力时，在相同离线维护条件下，由 owner 撤回 runtime 的精确函数权限：

```sql
REVOKE EXECUTE ON FUNCTION
  lmm_checkins.save_settings(bigint,text,boolean,text,text)
  FROM lmm_checkins_runtime;
```

随后恢复服务并确认 `writable=false`。保留函数、升级回执、策略历史和已发放记录；
不要删除策略表来回滚页面。撤回管理权限不等于停止已有奖励策略：需要停用领取时，应
使用明确的策略禁用或模块停用流程。旧二进制仍可读取现存契约版本 1 的策略数据。

## 验证证据与边界

2026-11-17 UTC 已执行的定向验证：

- 隔离 PostgreSQL 中验证升级事务回滚、提交、原意图恢复及错误意图拒绝。
- 验证真实策略保存、重建服务后读取、版本冲突、操作键重放、Core 上限检查，以及
  runtime 不能直接修改生效指针或删除历史。
- 验证完整 64 位整数金额和统一倍率不经过浮点数舍入。
- HTTP 测试覆盖实时会话校验、401/403 拒绝以及模块停用时管理路由仍可到达。
- Web 交互测试覆盖精确金额保存与回读、未知写入结果重试、冲突保留草稿、无效金额与
  上限限制、普通管理员拒绝，共 5 项通过。
- 七种语言文案通过翻译 helper 和同步脚本写入；相关 Web 文件的 oxlint 无警告、无错误。

源码测试入口为
[数据库与金额测试](../services/extensions/internal/modules/checkins/settings_test.go)、
[权限路由测试](../services/extensions/internal/webcompat/checkins_settings_test.go)和
[Web 交互测试](../apps/web/src/features/system-settings/general/checkin-settings-section.test.tsx)。
数据库测试只接受带 `lmm.synthetic=checkins-extension-test` 标记的全新专用测试库；
运行时需要私密 OWNER、RUNTIME 和 ADMIN 测试连接文件，不得指向生产库。

这些结果验证设置链路，不代表生产已应用升级，也不代表完成真实奖励领取。
生产版本、独立页面验收和完整 Web 类型检查的状态以本次部署记录为准。

## 推广与其他奖励入口

额度设置页改为现有能力的入口：`/redemption-codes` 管理 Core 奖励活动、预算和兑换码，
`/operations/sources` 管理推广链接与来源报表，签到入口使用本页描述的新接口。
旧的自动注册与邀请奖励、返佣门槛和惩罚字段没有当前在线策略消费者，已从该表单移除。
这一整理不创建新的奖励资金政策。

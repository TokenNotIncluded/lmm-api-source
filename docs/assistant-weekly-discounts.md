# 助手周优惠

周优惠记录一次判断，不等于余额到账。每个用户在同一 UTC 周只有一条决定；0% 表示本周已决定不发券，也占用本周机会。正折扣先保存为待领取，用户在自己的浏览器中确认后生成一张只供本人使用的单次充值优惠券。换对话、重复执行或重试领取均不能再获得一条决定或另一张券。

## 新决定的条件

新发放只面向当前启用的普通用户（platform_role=user）。已退役的信任等级不参与资格或额度判断。Core87→Core88 升级会建立默认关闭的 owner 政策行，兼容折扣上限为 10%；是否启用由站点整体激活材料确定。管理员不能新领周优惠。

Core 从原始持久化记录检查同一用户、个人账户和对话中的两次实质性交互。每条用户原话去掉首尾空白后至少有 4 个 Unicode 字符，并且有真实已结算的模型轮次和对应原始操作。压缩摘要、模型消息、传入的次数和同一轮的重复计费记录均不计为新的用户交互。

prepare_weekly_discount 只能由 Core 的持久化 worker 执行。当前操作须来自原浏览器 Session，并与原用户、账户、权限版本、主机策略、工具版本及参数摘要一致。状态查询和“是否已经评估”等追问不消耗本周机会。0% 决定和正折扣决定都在完成工具操作的同一事务中持久化。

## 接口

| 入口 | 参数 | 结果与权限 |
| --- | --- | --- |
| get_weekly_discount_status | 空对象 | 只读取当前用户本周已有决定；没有决定时明确返回 none，不推断资格 |
| prepare_weekly_discount | discount_percent 整数 0–99；reason 去空白后 2–240 字符 | 用当前 owner 政策限制实际发放比例；返回决定及浏览器地址，不返回私有码 |
| GET /api/user/auth/assistant/weekly-discounts | 可选 before_week | 当前用户的历史决定，按周倒序分页；关闭新发放后历史仍可读取 |
| GET /api/user/auth/assistant/weekly-discounts/{id} | 决定 ID | 当前用户的原理由、状态、可领取性；已领券的私有码只向本人浏览器展示 |
| POST /api/user/auth/assistant/weekly-discounts/{id} | claim=true、同源请求及当前 CSRF 凭据 | 再核验真实登录、原会话与当前政策后领取；重复领取返回原券 |

浏览器入口位于现有刷新 Cookie 覆盖的 /api/user/auth 路径。新决定领取时重新检查当前用户状态、普通用户角色、政策和 UTC 周截止时间。已经过期的历史待领取决定保留可读，不生成新的有效券。历史导入的原始记录没有伪造的原生 Session；它们通过 owner 源证据及当前目标用户映射保持归属。

## 历史迁移

旧站切换需要先完成真实身份和优惠券导入，再执行精确结构升级。已有别名导入仍使用固定 Core86 入口，顺序见 [旧邀请码迁移](legacy-affiliate-aliases.md)。完成别名导入后显式升级至 Core87，再用 core-assistant-weekly-discounts-v1 执行 Core87→Core88。

周优惠 owner 导入要求实际已提交的 c343 身份回执和对应 Core81 原券保全记录。所有源用户、周决定和原券来自同一份完整规范导出。已领取决定只关联原有券，保留原用户、原始整数对话编号、折扣、理由、周、创建时间和领取时间；导入不生成新券，也不调整账本。

先生成源预览：

```sh
python3 -B scripts/current_weekly_discounts.py source-preview \
  --support "$LMM_MIGRATION_SUPPORT" \
  --source-receipt "$LMM_SOURCE_RECEIPT" \
  --source-proof "$LMM_SOURCE_PROOF" \
  --output "$LMM_WEEKLY_PREVIEW"
```

输出路径须尚不存在。完整材料保存在私有文件，终端仅返回路径、哈希和数量摘要。支持目录与冻结 helper 的要求沿用 current_affiliate_aliases.py 的完整 SHA256 清单，周优惠模块另外绑定该共享模块本身的 SHA256。

prepare_owner、transaction_sql、resolve_sql 是离线 owner 驱动接口。生产准备材料还必须包含真实源停写证明、实际目标实例、已封存 Core88 manifest/catalog、全部表与序列的观察状态，以及精确绑定这些材料的审阅文件。未捕获的目标、预览身份映射和不完整源证据不能用于执行。

导入 SQL 使用一个 SERIALIZABLE 事务，共享迁移锁和完整读回。只有周决定、其源证据和本次导入回执发生写入；已有优惠券、其他表和序列保持准备时状态。重复提交只接受原始精确回执及完整决定集。提交结果未知时先执行 resolve_sql，检查原实例、原执行连接及已提交内容；不得自动重发事务。

## 权限与配置

运行角色没有历史导入表、原始源证据和升级回执权限，也不能更新周优惠政策。优惠券消费只读取 code_id 与 owner_user_id 的归属投影。原生 worker 只能插入决定所需字段，不能自行设置历史来源或领取结果；浏览器仅可更新领取 nonce，并调用限定为已有决定的发券函数。

默认政策为 enabled=false、maximum_percent=10。owner 的显式配置与请求中的共享政策锁相协调，因此准备和领取不会在同一事务中跨越两个不同政策。升级、导入历史和读取历史都不会自动启用新发放。

纯输入检查可按以下方式执行；真实 PostgreSQL、原浏览器 Session、已结算模型交互、券消费归属和并发重放检查需要另行使用匹配源码的验收产物：

```sh
PYTHONPATH="$LMM_MIGRATION_SUPPORT" \
  python3 -B scripts/test_current_weekly_discounts.py
```

## 隔离历史导入演练

数据库演练驱动只创建新的本地 PostgreSQL 实例。它实际执行冻结的 c343 身份事务，按精确 Core87 前驱安装周优惠结构，再检查原券关联、完整批次、共享迁移锁、状态变化拒绝、提交前断线、提交回执丢失和精确重放。

~~~sh
python3 -B scripts/check_legacy_weekly_discounts_pg.py \
  --source "$LMM_CORE_SOURCE" \
  --capture87 "$LMM_REVIEWED_CORE87_CAPTURE" \
  --support "$LMM_MIGRATION_SUPPORT"
~~~

capture87 目录须包含已审阅的 manifest87.json；驱动校验固定前驱摘要及源目录中全部组件的实际字节。源目录须是完整的对应源码树。测试使用虚构的周决定和预先存在的 Core81 原券记录；这验证周优惠导入机制，原券导入器和完整生产数据迁移需要各自的验收材料。该驱动的 SQL 结构安装也不能代替编译后的 Core87→Core88 owner 升级入口验收。

结果和日志写入新建的私有工作目录，驱动退出时停止自己创建的数据库。它没有已有数据库连接参数，不提供生产导入开关；生产执行仍使用前述经过完整源证据、停写边界和具体目标状态审阅的 owner 材料。

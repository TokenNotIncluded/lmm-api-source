# 请求限速与 Auto 分组继承

[文档目录](README.md) · [原生模型与计费交付](nonpayment-completion.md)

Core 使用 PostgreSQL 保存请求限速策略、全局 Auto 顺序与窗口计数。同一用户的多个
API Key 共用用户计数，同一分组的所有用户共用分组计数。管理员在现有
**安全设置 → 请求限速** 配置限额，在 **模型设置 → 分组** 保存全局 Auto 顺序；
两项配置各自使用版本号提交，互不覆盖。

实现入口是 [request_limits.rs](../services/core/src/request_limits.rs) 和
[request_limits.sql](../services/core/schema/request_limits.sql)。本页描述这些接口与
实际调用点，不代表生产升级、部署或独立验收已经完成。

## 计数与拒绝规则

限额的单位是**逻辑请求数／固定窗口**，与 Token、金额、余额和并发连接数无关。
只有配置的维度参与计数：用户上限按真实 `actor_user_id` 聚合，分组上限按实际选择的
路由分组聚合。跨组重试保留同一个逻辑请求 ID，用户在同一窗口内只计一次，每个实际
尝试的分组各计一次。另一个用户使用相同请求 ID，仍会计入共享分组限额。

数据库函数先检查两个维度，再同时增加需要消费的计数。任一维度超额时，另一维度
也不会增加。数据库事务锁与数据库时钟在多个 Core 进程间共享；进程重启不会清空
当前计数。数据库读取或提交结果无法确认时，请求不能获得放行。

公共模型路径在预扣资金和发送上游请求前检查限额。请求放行后即使上游失败，也不会
退回已经消费的次数。个人助手把限速检查放在现有资金事务中，限速计数与有效的工作者
预扣一起提交；该事务回滚时，其计数也回滚。已经完成的图片结果查询、报价预览、
模型目录读取不产生新的模型执行计数。

窗口按 Unix 秒对齐，以 PostgreSQL 时钟计算：
`window_started = now - now % window_seconds`。例如每 60 秒允许 100 次时，窗口末尾
的 100 次与下一个窗口开头的 100 次可以相距很近；这不是滑动窗口保证。公共模型
HTTP 接口超额时返回 `429`，并带正整数秒的 `Retry-After`，表示当前窗口剩余等待时间。
内部个人助手 RPC 使用 `resource_exhausted` 与 `retry-after` 元数据；异步 agent
将被限速的轮次记为既有的 `rejected` 状态，没有 `final_text`，不会将其伪装成完成，
也不会继续发送后续模型轮次。同一轮次再次提交仍返回原拒绝状态。

修改次数上限不会清除同一窗口的已有计数。修改窗口时长会使用该时长对应的窗口计数；
如果切回仍未过期的旧时长，之前的计数仍存在。过期桶及其请求去重记录在后续已放行
请求中按批清理。策略修订号不作为计数桶的一部分，不能通过仅保存同一策略重置额度。

## 管理请求限额

`GET /core/v1/admin/request-limits` 返回当前策略，例如新安装默认值：

```json
{
  "version": 1,
  "revision": 1,
  "policy": {
    "enabled": false,
    "window_seconds": 60,
    "user_limit": null,
    "group_limits": {}
  }
}
```

字段语义如下。配置由数据库统一管理，不另设 TOML 覆盖源，也不读取旧的
`ModelRequestRateLimit*` 控制台选项作为运行计数或策略来源。

| 字段 | 单位与范围 | 关闭或省略的语义 |
| --- | --- | --- |
| `enabled` | 布尔值 | `false` 跳过所有请求限速计数与限制 |
| `window_seconds` | 整数秒，1–86400 | 必须给出有效时长，`0` 无效 |
| `user_limit` | 每用户每窗口的请求数，1–100000000 | `null` 不设用户维度上限；表单留空写为 `null` |
| `group_limits` | 精确分组名到请求数的对象，每项 1–100000000，最多 1024 项 | 未列出的分组不设分组维度上限；删除对象项即可取消该组上限 |

启用策略时至少需要一个有效上限。数值 `0`、负数、小数、旧式 `[total, success]`
数组都不是有效的限额。分组名遵循路由现有的精确 Unicode 标签规则，不自动修剪、
大小写折叠或规范化；配置一个分组限额不会授予任何人该分组的访问权限。

`PUT` 使用最近一次 `GET` 的 `revision` 作为 `expected_revision`。例如每用户每分钟
60 次、`default` 分组全体用户共享每分钟 300 次：

```bash
curl --fail-with-body \
  -H "Authorization: Bearer ${LMM_ADMIN_ACCESS_TOKEN}" \
  "${LMM_CORE_URL}/core/v1/admin/request-limits"

# 先读取当前 revision，并将下面的 1 替换为实际值。
curl --fail-with-body -X PUT \
  -H "Authorization: Bearer ${LMM_ADMIN_ACCESS_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data-binary @- \
  "${LMM_CORE_URL}/core/v1/admin/request-limits" <<'JSON'
{
  "expected_revision": 1,
  "policy": {
    "enabled": true,
    "window_seconds": 60,
    "user_limit": 60,
    "group_limits": { "default": 300 }
  }
}
JSON
```

成功返回新的完整快照与递增的 `revision`。版本冲突返回 `409`，管理员应重新读取，
核对自己的草稿后再提交。设置页保留冲突后的草稿，不自动重放写入；“重新读取当前配置”
会用当前服务器值替换草稿。非法策略返回 `400`。

读写接口均要求当前有效的 **superadmin Session**。普通用户、普通管理员、API Key、
付费服务等级都不能替代此权限。响应使用 `private, no-store`；每次成功保存同时写入
`policy_events`，记录操作者、配置类型、修订号与内容。

## 全局 Auto 顺序与 API Key

`GET /core/v1/admin/routing/auto-groups` 返回独立的顺序快照：

```json
{ "version": 1, "revision": 1, "groups": [] }
```

全局 `groups: []` 表示按名称顺序使用该用户当前已授权、已启用的全部分组。
非空全局列表决定继承模式的候选组与顺序，不会把未列出的组自动附加到末尾。每次
请求会重新读取该列表，再过滤未授权或已停用的组。

管理员可使用同样的 Bearer Session 保存顺序；数组中的分组名必须唯一，最多 1024 项：

```bash
curl --fail-with-body -X PUT \
  -H "Authorization: Bearer ${LMM_ADMIN_ACCESS_TOKEN}" \
  -H 'Content-Type: application/json' \
  --data-binary @- \
  "${LMM_CORE_URL}/core/v1/admin/routing/auto-groups" <<'JSON'
{
  "expected_revision": 1,
  "groups": ["priority", "default"]
}
JSON
```

其中 `expected_revision` 必须来自 Auto 接口的当前快照，不是限速策略版本。示例中的
组名仅是配置值：不存在、停用或用户无权使用的组不会进入该用户的运行候选。

API Key 使用 `group: "auto"` 时，`auto_groups: []` 是持久化的**继承标记**。
服务器保存空列表，每次请求即时解析全局顺序；不会在创建或编辑时把当前全局列表
固化到 Key 上。已有非空 `auto_groups` 始终作为自定义列表处理，包括历史导入的固化
列表；本轮不擅自把它们转换为继承。自定义列表保留顺序，并在请求时过滤无权或停用的
分组。自定义列表上限为 16，继承结果不按这个上限截断。

上述候选过滤适用于仍然有效的凭证。停用路由发布中的分组会影响可用候选；撤销用户
分组授权则按既有身份规则递增用户 `auth_version`，使此前签发的 Session 和 API Key
失效，后续请求先返回 `Unauthorized`。凭证失效不会改写 Key 保存的 `auto_groups`，
也不会把非空自定义列表转换为继承。

`GET /api/token/auto-groups` 返回当前用户可见的全局 Auto 顺序和 `max_count: 16`。
`GET /api/user/self/groups` 继续返回全部授权且启用的可选组，方便显式选组或保存自定义
列表。全局 Auto 改序不会改写自定义列表，也不会撤销用户对其他组的显式使用权限。

如果一个已有 Dashboard Key 的 Auto 候选被过滤为空，请求拒绝继续路由，不能退回主机
为原生 Key 配置的分组映射。只有没有 Dashboard 路由设置的原生 Key 才使用该映射。
跨组重试还要求 Auto Key 的 `cross_group_retry` 显式为 `true`；它与继承模式、排序或
限速开关是不同的设置。身份处理见
[identity/compat.rs](../services/core/src/identity/compat.rs)。

## 实际执行入口

以下位置已经调用同一套 admission 函数；限额使用实际执行分组，不能通过换一个公共
模型端点避开用户共享额度。内部重试沿用同一逻辑请求 ID，结果查询不新建执行次数。

| 路径 | 限速调用位置 |
| --- | --- |
| 普通文本网关，包括 Chat、Responses、Messages、Gemini 的已有路径 | [model_http.rs](../services/core/src/model_http.rs) 预扣前检查；[routing_session.rs](../services/core/src/model_http/routing_session.rs) 的 `record_selected_dispatch` 在每次上游派发前检查实际分组 |
| `/v1/embeddings`、`/v1/rerank` | [input_api.rs](../services/core/src/model_http/input_api.rs)，建立资金请求前 |
| `/v1/tools/search`、`/v1/tools/fetch` | [tool_api.rs](../services/core/src/model_http/tool_api.rs)，建立资金请求前 |
| `/v1/systemone`、`/typesafe/v1/systemone` | [systemone.rs](../services/core/src/model_http/systemone.rs)，建立资金请求前 |
| `/v1/images/generations`、`/v1/images/edits` | [image_api_execution.rs](../services/core/src/model_http/image_api_execution.rs)，冻结实际组后、供应商准备及预扣前 |
| 已确认的绘图工具执行 | [image_execution.rs](../services/core/src/model_http/image_execution.rs)，验证原授权后、供应商准备及预扣前 |
| 普通个人助手及 agent 模型轮次 | [charging/assistant.rs](../services/core/src/billing/charging/assistant.rs) 的 `reserve_assistant`，与资金事务共用连接 |

这张表不宣称新增音频、Realtime、视频或完整 Responses 能力。没有实现的模型接口
不会因为增加限速配置而成为可用能力。

## 安装权限与验证边界

新组件 `core_request_limits` 使用单独的 schema 文件。runtime 只需要 schema `USAGE`、
`policy` 与 `auto_policy` 的 `SELECT`，以及以下三个受限函数的 `EXECUTE`：

```text
core_request_limits.admit(BIGINT, TEXT, TEXT)
core_request_limits.write_policy(BIGINT, BOOLEAN, INTEGER, INTEGER, JSONB, BIGINT)
core_request_limits.write_auto_groups(BIGINT, TEXT[], BIGINT)
```

计数桶、去重记录和配置事件不授予 runtime 直接写权限。函数由安装所有者创建，使用
固定 `search_path`，撤销 PUBLIC 权限。正常服务启动不会建表；已有数据库必须按显式
升级流程完成新组件安装与权限检查后再启动新版本。不要把本页的 API 示例当作数据库
升级脚本。

测试源码覆盖权限、CAS、用户多 Key 聚合、跨用户组共享、并发计数、重试去重、双维度
原子拒绝、过期窗口及存储失败，并在原生接口和助手路径断言拒绝时无上游发送、无资金
预扣。入口是 [request_limits.rs 测试](../services/core/tests/request_limits.rs)、
[identity_compat.rs 测试](../services/core/tests/identity_compat.rs)、各模型适配器相邻测试
及 [assistant_model.rs 测试](../services/core/tests/assistant_model.rs)。这些测试是否已执行、
具体通过结果与未覆盖项，以本轮交付记录为准，不能从测试源码的存在推定验收通过。

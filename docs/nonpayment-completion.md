# 非支付功能交付指南

[开发与测试](development.md) · [配置路径](filesystem-layout.md) · [架构](core-migration.md)

本轮补齐供应商用量进入 Core 账单的若干实际路径、两个 RAG 接口，以及原生用量的管理查询和筛选导出。本文记录第一轮提交的行为、配置和验证结果，不把源码存在视为完整验收。第一轮不改变支付流程或数据库 schema；随后增加的请求限速、路由回退、按次计费与运维组件，见[第二轮交付记录](request-operations-completion.md)及其 29 → 31 离线升级要求。

## 1. 交付范围与证据

| 能力 | 当前行为 | 实现与测试入口 |
| --- | --- | --- |
| DeepSeek 缓存与 reasoning | 读取 cache hit/miss，校验分区一致性；保留 reasoning effort；推理 token 不重复计费 | [回执解码](../services/core/src/relay/decode.rs)、[请求转换](../services/core/src/relay/request.rs)、[真实账务测试源码](../services/core/src/model_http/prepared_financial_tests.rs) |
| DeepSeek Search/Fetch | 独立 HTTP 接口，按完整供应商回执确定成功请求数，使用冻结的每次请求价格 | [接口](../services/core/src/model_http/tool_api.rs)、[价格与回执](../services/core/src/model_http/tool_price.rs) |
| Gemini Google Search | 仅支持有明确上界的 `grounded_request_v1` 合同；一次逻辑请求最多计一次 grounding 费用 | [合同与价格](../services/core/src/model_http/grounding_price.rs)、[供应商绑定](../services/core/src/model_http/provider_policy.rs)、[账务桥接](../services/core/src/model_http/billing.rs) |
| Embeddings / Jina Rerank | 使用原生接口，校验完整响应和供应商输入用量，再走 Core 预留、结算及恢复路径 | [接口](../services/core/src/model_http/input_api.rs)、[请求与回执](../services/core/src/model_http/input_contract.rs)、[PostgreSQL 测试源码](../services/core/src/model_http/input_api_tests.rs) |
| 原生用量管理 | 管理员列表、统计、CSV；个人按相同筛选导出自己的原生记录；缓存分项可见 | [查询投影](../services/core/src/usage.rs)、[CSV](../services/core/src/usage/export.rs)、[Web 页面](../apps/web/src/features/usage-logs/index.tsx)、[权限与筛选测试源码](../services/core/src/usage/index_tests.rs) |

## 2. 配置入口、价格单位与冻结

模型配置由安装方的 TOML 文件提供。主配置的 `[model].config_file` 指向模型 sidecar；外部供应商使用 HTTPS 和 `local_http = false`。配置字段以 [Core 加载器](../services/core/src/config.rs)、[路由类型](../services/core/src/routing/config.rs)和[价格类型](../services/core/src/routing/pricing.rs)为准。以下内容是可替换参数的配置模板，不是供应商官方价格，也不是已部署的线路。

```toml
# 合并到已有 core.toml；其余身份、数据库和服务配置沿用现有安装。
[model]
config_file = "/absolute/private/core-model.toml"
local_http = false
```

价格字段使用整数，必须区分 token 单价和请求单价。

| 字段 | 单位与含义 |
| --- | --- |
| `prices.rules[].rates.input/output/cache_read/cache_write` | 每一百万 token 的 micro-USD；`1000000` 表示 USD 1 / 百万 token |
| `prices.rules[].rates.request` | 每次逻辑请求的 micro-USD；Search/Fetch 将它解释为每次成功请求价格 |
| `external_models[].grounding.usd_micros_per_grounded_request` | 每次 grounded request 的 micro-USD，独立于 token 价格 |
| `group_multiplier_ppm`、`speed_multiplier_ppm` | 百万分比；`1000000` 为 1 倍，`500000` 为 0.5 倍 |
| Core credits | 当前换算为 USD 1 = 500000 credits；最终金额向上取整到整数 credit |

请求先绑定授权分组、目标、凭据 revision、供应商合同和价格版本，再预留费用。实际回执只能按这份冻结价格结算；恢复不会重新读取当前价。工具费用及 grounding 合同写入已有冻结价格和用量事件，不增加 DDL。更新线路、价格或供应商语义时应使用新的版本，并保留能读取已有冻结字段的程序完成恢复；不要通过修改旧记录或删除用量证据实现回滚。

下面是一条 Search 线路的完整模型 sidecar 示例。`provider.example.invalid`、私密文件路径、合同 revision 和示例价都必须替换成经过核对的安装参数；凭据文件保存供应商密钥，不能把密钥写进 endpoint。`key_groups = []` 不授予调用权限，实际调用者仍需已有身份、模型密钥和对应分组授权。

```toml
version = 1
input_token_ceiling = 8192
lease_seconds = 60
timeout_seconds = 30
max_output_tokens = 1024
key_groups = []
credentials = [
  { reference = { id = 101, revision = 1 }, secret_file = "/absolute/private/search-key" },
]

[[external_models]]
upstream_id = 101
upstream_model = "deepseek-search"
protocol = "deepseek_search"
endpoint = "https://provider.example.invalid/v1"
max_input_tokens = 0
max_output_tokens = 0
contract_revision = "reviewed-search-contract-v1"

[routing]
version = 1
groups = [{ name = "default", enabled = true }]
aliases = []

[[routing.upstreams]]
id = 101
endpoint = "https://provider.example.invalid/v1"
protocol = "deepseek_search"
credential = { id = 101, revision = 1 }
enabled = true

[[routing.routes]]
id = 101
model = "deepseek-search"
group = "default"
enabled = true
speeds = ["standard"]
failover = { max_attempts = 1, retry_on = [], allow_replay_after_send = false }
targets = [{ id = 101, upstream_id = 101, upstream_model = "deepseek-search", enabled = true, priority = 0, weight = 1, speeds = ["standard"], payload = "adapt", timeout_ms = 30000 }]

[prices]
version = 1

[[prices.rules]]
id = 101
model = "deepseek-search"
group = "default"
speed = "standard"
billing_mode = "tokens"
rates = { input = 0, output = 0, cache_read = 0, cache_write = 0, request = 750 }
group_multiplier_ppm = 1000000
speed_multiplier_ppm = 1000000
```

这里 `request = 750` 仅用于演示：无折扣、两个倍率均为 1 时，一次成功请求为 USD 0.000750，即 375 credits。代码不内置这个供应商价格。Search/Fetch 的 token 单价必须全为零，并且不接受 `tiered_decimal` 或 `upstream_cost` 模式。新增 Fetch 时使用独立 ID，将协议改为 `deepseek_fetch`、模型改为 `deepseek-fetch`，另设实际请求价；不要用单条规则覆盖已有配置。

不同操作必须使用匹配的原生协议与供应商上下文，不能仅改模型名称。

| 操作 | `protocol` | `external_models` 上界 | 价格限制 |
| --- | --- | --- | --- |
| DeepSeek 聊天 | `openai_chat` | 已核对的输入、输出上界 | 按实际输入、缓存读和输出价格配置 |
| Search / Fetch | `deepseek_search` / `deepseek_fetch` | 输入、输出都为 `0` | 仅 `rates.request` 可非零 |
| Embeddings | `openai_embeddings` | 输入是整个 batch 的上界；输出为 `0` | 标量 token 价格，仅 `rates.input` 可非零 |
| Jina Rerank | `jina_rerank` | 输入是 query 与整个候选集合这次操作的上界；输出为 `0` | 标量 token 价格，仅 `rates.input` 可非零 |
| Gemini grounding | `gemini` | 已核对的输入、输出上界；额外最多一次 grounding | 标量 token 价格，`rates.request = 0`、`rates.cache_write = 0` |

输入上界是供应商合同保证，不是本地估算 token 数；不得把单个 embedding 或单个候选文档的限制充当整次请求上界。目标使用 `payload = "adapt"`，`timeout_ms` 不小于全局 `timeout_seconds × 1000`，上下文的 `upstream_id/upstream_model/protocol/endpoint` 必须与实际目标绑定一致。现有运行授权仍会检查资金、密钥限制和分组。

## 3. 供应商回执如何入账

**DeepSeek 缓存使用明确计数，reasoning 保持供应商语义。** `prompt_cache_hit_tokens` 映射为缓存读取；它与 `prompt_tokens_details.cached_tokens` 同时存在时必须一致。存在 hit、miss 和总输入时，必须满足 `hit + miss = prompt_tokens`，对应官方 [Context Caching](https://api-docs.deepseek.com/guides/kv_cache/) 和 [Chat API 用量定义](https://api-docs.deepseek.com/api/create-chat-completion/)。缺失 cache hit 不会按模型名称推定为零；只有对应 `external_models` 合同明确设置 `missing_cached_tokens_is_zero = true` 才采用缺失即零规则。缓存写入的缺失约定由独立的 `missing_cache_creation_tokens_is_zero` 控制。

Chat 的 `reasoning_effort` 以及 Responses 的 `reasoning.effort` 接受 `none/minimal/low/medium/high/xhigh/max`，转发时保留原值，不把别名归一化为另一档。供应商是否接受某一档仍取决于选中的端点合同。`completion_tokens_details.reasoning_tokens` 是输出总量的子集，不能再加到 `completion_tokens` 上。比如总输入 100、cache hit 80、cache miss 20、输出 10、其中 reasoning 7，应结算普通输入 20、缓存读 80、输出 10。

官方 [Chat API](https://api-docs.deepseek.com/api/create-chat-completion/) 与 [Thinking Mode 指南](https://api-docs.deepseek.com/guides/thinking_mode/)用于核对供应商控制语义；后者还列有 `ultra` 别名，本轮 Core 白名单尚未接受它。本文所列 effort 支持不等于完整移植供应商所有 thinking 扩展。

**Search/Fetch 使用成功请求回执，不生成虚构 token。** Search 的完整 HTTP 200 JSON 必须有 `results` 对象数组；非空计 1，空数组计 0。Fetch 的完整 HTTP 200 JSON 必须有字符串 `content`；去空白后非空计 1，否则计 0。符合传输校验的完整 4xx/5xx 回执计 0，操作结果保留为 `failed`；空结果的 HTTP 200 则是 `completed`。重启恢复按持久化的 HTTP 状态恢复同一结果。缺失字段、错误结构、超时或不完整响应保留待核对状态。请求费用冻结在预留中，完整回执的状态、计数与摘要进入持久化证据。

成功请求的供应商语义与接口路径来自 [DeepSeek Search & Fetch 指南](https://api-docs.deepseek.com/guides/search/)。供应商价格、免费期和生效时间可能调整，启用时应核对该页并填写安装方实际费率；模板值不自动追踪官网，也不会按日历自动切换。Core 在发送后超时而缺少可验证回执时保留预留，不能仅凭供应商“超时不计费”的文字推定该请求已获零费回执。

**Gemini 当前只覆盖每次 grounded request 的有界合同。** 在对应 `[[external_models]]` 内显式增加以下子表；示例价仍须替换。没有该合同、选中非 Gemini 目标，或选择逐 query 的计费语义时，请求被拒绝。

```toml
[external_models.grounding]
mode = "grounded_request_v1"
tool_input = "exclude_search_retrieval_v1"
usd_micros_per_grounded_request = 35000 # 仅演示：USD 0.035 / grounded request
```

客户端使用原生 Gemini `tools: [{"googleSearch": {}}]`；本轮不接受与函数声明等其他工具混用。回执保留 `groundingMetadata`；非空 `webSearchQueries` 或带有效 URI 的 web grounding chunk 建立“发生 grounding”的证据。多条 query、多块流式回执只更新同一次请求的累计 0/1 计数，不相加。明确的空 query 数组且无 web grounding 可建立 0；完全缺失这类证据仍是未知，不能按零结算。

`usageMetadata.toolUsePromptTokenCount` 单独保留为 `tool_input_tokens`。上述 `exclude_search_retrieval_v1` 合同明确不把这些搜索检索 token 再加入可计费输入；普通输入、缓存和输出仍来自供应商用量。grounding 费用与 token 费用合并后应用冻结倍率和适用折扣，最终只做一次 credit 向上取整。**按搜索 query 数收费的 Gemini 合同尚未实现**；不能根据模型名或 `webSearchQueries.length` 宣称已支持。

截至 2026-10-30 核对的官方 [Google Search grounding 指南](https://ai.google.dev/gemini-api/docs/google-search)明确区分 Gemini 3 及更新模型的逐 query 计费与 Gemini 2.5 及更早模型的逐 prompt 计费；实际费率和模型条件以 [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing) 为准。因此本轮 `grounded_request_v1` 不能用于声称已完成 Gemini 3+ 的标准 Search 计费，也不能把不相符的供应商合同手动标成该模式。代码不靠模型名称猜测合同。

**Embeddings/Rerank 必须拿到完整输入用量。** Embeddings 支持字符串、字符串数组、token ID 数组及其批量形式，最多 2048 项；支持 `encoding_format` 为 `float/base64` 和可选 `dimensions`。回执须覆盖所有索引、维度一致、数值有限，并且 `usage.prompt_tokens == usage.total_tokens`。Jina Rerank 接受非空 `query`、最多 2048 个字符串 `documents`，以及合法的 `top_n/return_documents`；使用 `usage.total_tokens`，若同时存在 `prompt_tokens`，两者必须一致。它们的输出计费用量固定为 0，不把结果数量或向量维度转换为 token。

这些新原生接口使用模型 API key；现有生成 API 的 OAuth scope 不自动授予调用权。调用后的工作由 Core 持有，客户端断开不取消账务责任；同一 `Idempotency-Key` 绑定同一密钥下的逻辑请求，重复调用不重新发供应商请求。恢复依赖已有持久化回执，缺失证据时保留原预留供核对。

## 4. 调用示例

以下命令使用已经配置好的 `BASE_URL` 和模型 `API_KEY`；本文没有执行这些请求。`embedding-model`、`rerank-model`、`deepseek-chat`、`grounded-model` 是需要配置的本地路由名，并非新增供应商模型名称。`curl -D -` 可读到 Core 返回的 `x-request-id`，用于原生日志查询。

```sh
curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1/tools/search" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"query":"Core billing","count":5,"need_content":true}'

curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1/tools/fetch" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"url":"https://example.com/"}'

curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1/embeddings" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"model":"embedding-model","input":["first text","second text"],"encoding_format":"float"}'

curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1/rerank" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"model":"rerank-model","query":"billing","documents":["billing receipt","routing setup"],"top_n":1,"return_documents":true}'

curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"model":"deepseek-chat","messages":[{"role":"user","content":"Explain caching."}],"reasoning_effort":"max","max_tokens":128}'

curl --fail-with-body --silent --show-error -D - "$BASE_URL/v1beta/models/grounded-model:generateContent" \
  -H "Authorization: Bearer $API_KEY" -H 'Content-Type: application/json' \
  --data '{"contents":[{"parts":[{"text":"Find documentation on prompt caching."}]}],"tools":[{"googleSearch":{}}],"generationConfig":{"maxOutputTokens":128}}'
```

Search 的 `query` 最长 128 字符，`count` 为 1–50；另支持 `ori_question`、IP 字符串 `localization` 和布尔 `need_content`。Fetch 接受最长 8192 字符、带主机且不含账号密码的 HTTP(S) URL，由配置的供应商执行抓取。Search/Fetch 不从请求体读取模型、目标或价格。Gemini 流式调用使用 `:streamGenerateContent?alt=sse`，普通 Chat 使用 `"stream": true`；两种形式走相同冻结计费语义。

## 5. 原生用量、缓存统计与 CSV

管理员“常规日志”默认显示原生用量，并可切换历史来源。列表、统计和导出使用相同已应用筛选；切换来源时清除原生接口不支持的渠道和上游请求 ID。历史 `/api/log`、`/api/log/stat`、`/api/finance/export` 保留，历史导出不替代原生导出。

| 身份与用途 | 入口 |
| --- | --- |
| 管理员原生请求列表 | `GET /core/v1/usage/admin/logs` |
| 管理员原生汇总 | `GET /core/v1/usage/admin/stats` |
| 管理员原生 CSV | `GET /core/v1/usage/admin/export.csv` |
| 个人原生 CSV | `GET /core/v1/usage/self/export.csv` |
| 个人原生列表、汇总 | `GET /api/log/self`、`GET /api/log/self/stat` |
| 原生看板聚合 | `GET /api/data`、`GET /api/data/self` |

这些入口使用 Web 会话身份，管理员入口额外核验管理权限。个人范围按请求操作者及个人密钥所属账户限定；团队出资的个人密钥记录仍在个人范围，团队所属密钥不因此暴露。列表、统计和 CSV 都从同一只读快照校验实际结算、退款与用量证据；普通用户不能通过 `username` 扩大范围。

共同筛选包括 `start_timestamp/end_timestamp`（Unix 秒）、`model_name`、`token_name`、`group`、`request_id`；管理员可加 `username`。最大时间窗 31 天，缺省为最近 31 天。列表支持 `p` 和 `page_size`，每页最多 100；**CSV 不接受分页参数**，导出所选窗口内的完整匹配结果。非零 `channel`、非空 `upstream_request_id` 被拒绝。单次最多 100000 条请求，每条最多 2048 条命令或退款证据，CSV 最大 32 MiB；超出边界报错，不静默截断成功文件。

```sh
# WEB_ACCESS_TOKEN 是已有登录会话的访问令牌；REQUEST_ID 来自模型响应。
curl --fail-with-body --silent --show-error --get \
  "$BASE_URL/core/v1/usage/self/export.csv" \
  -H "Authorization: Bearer $WEB_ACCESS_TOKEN" \
  --data-urlencode "request_id=$REQUEST_ID" \
  --output native-usage.csv

# 管理员令牌可用相同筛选查询；额外按 username 限定。
curl --fail-with-body --silent --show-error --get \
  "$BASE_URL/core/v1/usage/admin/stats" \
  -H "Authorization: Bearer $ADMIN_WEB_ACCESS_TOKEN" \
  --data-urlencode 'username=example-user' \
  --data-urlencode 'model_name=deepseek-search'
```

账单列区分预留、已结算总额、退款和净消费；未结算请求保留状态及预留金额，最终用量和消费留空。CSV 还包含价格版本、出资账户类型、钱包/订阅来源、缓存 TTL、图像数量及工具维度，并用 `outcome` 与 `provider_http_status` 区分操作结果和账务状态。Search/Fetch 的 `pricing_dimension` 为 `tool`、`billing_method` 为 `per_successful_request`，日志和 CSV 的 `tool_kind` 均为 `deepseek_search/deepseek_fetch`。逐请求日志的 `tool_requests` 与 CSV 的 `successful_tool_requests` 表达同一 0/1 成功计费次数；未结算时保持空值。Gemini 单列 `grounded_requests`，逐请求 `other.tool_input_tokens` 与 CSV `tool_input_tokens` 保留检索输入计数，不折算成额外计费 token 或图像。

列表和详情直接显示成功工具次数，包含真实的 0；待核对请求显示未知。Gemini 详情将 grounding 次数和“搜索检索 token（不计费）”与普通输入、输出分开。原生看板桶增加 `tool_requests/grounded_requests`：不适用的维度为 `null`，适用且测得零为 `0`。纯工具桶的 `token_used` 仍为 `null`，前端依据非空的工具次数识别有效记录，既不丢弃零次工具请求，也不使混合数据中完整的文本 token 小计失效。相关接线见[日志数量展示](../apps/web/src/features/usage-logs/lib/usage-quantity.ts)和[看板桶校验](../apps/web/src/features/dashboard/lib/usage-bucket.ts)。

缓存统计显示 `cache_read_tokens`、`cache_write_tokens` 和 `cache_write_5m_tokens/cache_write_1h_tokens/cache_write_12h_tokens`。某个写入维度缺少完整证据时，该汇总保持 `null`，页面显示 `—`；不会用零填充缺失值。预留中的记录不进入最终缓存消费汇总。统计趋势若数据不完整，继续显示不可用，不绘制误导性柱图。CSV 采用白名单字段并处理电子表格公式前缀，不含供应商原始响应、凭据、IP 或任意诊断对象。

## 6. 验证记录与边界

原生日志与导出基础改动的 Web 测试 **58/58 通过**，后续工具/grounding 接线及相关回归 **73/73 通过**。两批有重叠，不把它们相加为独立用例总数。前者覆盖 API 与导出参数、统计数据、统计恢复、筛选与下载交互、隐藏/不完整趋势；后者补充工具 0/1/未知、grounding 明细、看板桶与混合 token 汇总。工具接线前实际复现的 7 个失败和修复后通过结果分别留存。

管理员趋势旧用例的 fixture 标记 `truncated = true` 却期待柱图，已保留该标记并改为断言不可用且无柱；个人完整数据的 12 柱与点击断言保留，生产趋势逻辑未修改。原基线失败证据保留。

Core 的两批定向检查去重后有 **229 个用例最终通过**。首批运行 121 个用例，116 通过、5 个 Embeddings 相关用例失败。合法小数回执被 `arbitrary_precision` 与 untagged 反序列化的组合拒绝；改为直接解析数组/字符串后，输入接口及共享计费、恢复、缓存证据共 115 个用例全部通过。原有索引、维度、有限数值和 usage 一致性断言均保留，并增加整数、小数、指数及非法值回归。两次整库运行期间 Rust 输入均冻结，原始失败日志、修复后结果和逐文件摘要一起保留；未改变的其他通过项复用首次结果。

账务检查使用 **PostgreSQL 18.6 隔离数据库、Core 的真实身份和账本、受限 runtime 角色，以及真实本地 HTTP 供应商 fixture**。DeepSeek JSON/SSE 均验证 48 credits；Gemini 分别验证合并后一次舍入的 17 与 12 credits。Search/Fetch 覆盖成功、空结果、JSON/纯文本错误、并发去重、缺失回执留款与冻结原价恢复；错误回执恢复后仍为 `failed`。Embeddings/Rerank 的权限、资金拒绝、并发和恢复，以及管理员/个人原生导出的筛选权限均已运行通过。供应商响应和资金为隔离测试数据，没有进行真实供应商付费对账。

| 检查 | 当前记录 |
| --- | --- |
| Web 受影响用例 | 58/58 通过 |
| 日志页导航类型修复后的定向 UI 检查 | 筛选/导出与隐藏趋势共 11/11 通过；导航改为绑定日志页路由，未改搜索 schema |
| 工具/grounding 前端与相关回归 | 7 个测试文件 73/73 通过；受影响 15 文件定向格式化及 lint 通过 |
| 原生目录、按次价格与调用示例 | 两组测试 18/18 通过；示例补括号后单项复测 1/1；7 文件格式/lint 通过，未改的 12h 测试保留原 lint warning |
| 新增工具/grounding 文案 | 3 个键覆盖 7 种语言，翻译同步 missing/extras 为 0；未声称已做浏览器布局验收 |
| Web 整体 typecheck | `bun run typecheck` 最终 exit 0；首次 TS2322 及修复记录保留 |
| Web 生产构建 | `bun run build` 与 `bun run bundle:check` 通过；首屏 gzip 为 550627 bytes，上限 786432；使用 `nonpayment-selftest` 构建标识，没有发布此构建 |
| Rust 编译与定向测试 | 两次完整库编译后的定向测试累计 229 个不同用例最终通过；实际命令见交付日志 |
| Rust 公共接口调用方 | `cargo check --locked --offline --all-targets` 最终 exit 0；补齐两处既有集成测试别名构造器中的可选字段，未修改测试行为 |
| Rust 格式 | 54 个改变的 Rust 文件全部通过 rustfmt 检查 |
| 隔离 PostgreSQL 账务测试 | 已实际运行通过；覆盖上文的 HTTP、身份、预留、结算、恢复和原生用量链 |
| 真实浏览器、真实供应商及部署验收 | 未执行；全仓全部测试及独立验收也未执行 |

测试使用 Rust 1.99.0、已锁定依赖和基于 `7abe06d5d369b2ccefe4364099450bfae4534953` 的本轮源码。下列名称可定位关键回归；`DATABASE_URL` 必须指向独立可销毁 PostgreSQL。交付日志保留实际批量命令、退出码、运行前后的源码摘要和失败修复记录。

```sh
cargo test --manifest-path services/core/Cargo.toml --locked --lib deepseek_cache_receipt_reaches_native_bill_for_json_and_stream
cargo test --manifest-path services/core/Cargo.toml --locked --lib gemini_grounding_frozen_request_fee_reaches_ledger_and_native_bill
cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::input_api::tests::
cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::tool_api::pg_tests::
cargo test --manifest-path services/core/Cargo.toml --locked --lib usage::export
cargo test --manifest-path services/core/Cargo.toml --locked --lib native_admin_and_self_exports_share_filters_and_authorization
```

工具前端这一批的复跑命令如下；测试源码与调用方一起保存在仓库中。

```sh
cd apps/web
bun test src/features/usage-logs/components/__tests__/native-tool-usage.test.tsx \
  src/features/usage-logs/components/__tests__/native-image-usage.test.tsx \
  src/features/usage-logs/components/__tests__/request-summary.test.tsx \
  src/features/usage-logs/lib/statistics.test.ts \
  src/features/dashboard/lib/usage-bucket.test.ts \
  src/features/dashboard/lib/stats.test.ts \
  src/features/dashboard/lib/model-usage.test.ts
```

## 7. 尚未完成的产品闭环

本轮不代表所有非支付能力已完成。以下仍需独立实现和验收：

- **渠道和定价控制面：** 管理员渠道增删、启停、在线测试、多密钥、排序，以及模型价格、分组费率、汇率同步和预览后提交的完整界面与 Core 写入链。已有路由底层、只读目录或价格草稿不等于这些操作已完成。
- **钱包和订阅管理：** 兑换码生成、管理、兑换到入账的完整闭环，以及用户和管理员的订阅管理接口。
- **更多模型接口：** 音频、Realtime、视频和更完整的 Responses 能力；Gemini 按 query 计费也仍在此列。
- **账户与通知：** 更完整的用户管理、Passkey、自助访问令牌管理、通知偏好和低余额提醒。

已有 OpenAI 12 小时缓存解析与结算、SMTP 邮件、订阅权益消费和 Go 扩展宿主继续保留，不应重新列为本轮从零补齐的缺口。原生用量页展示结构化账务明细，也不意味着开放供应商原始请求/响应或所有排障功能。

交付核对按五个问题记录：本轮贡献是否对应真实接口（第 1 节源码入口）；配置能否据字段复现（第 2、4 节）；通过项是否有实际结果（第 6 节及交付日志）；未测与未完项是否可辨认（第 6、7 节）；资金设计是否保持权限、冻结价格和未知回执边界（第 2、3、5 节）。本轮结论为：**自测通过；独立验收未执行**。实际供应商合同与生产环境仍需使用该环境自己的回执核对。

# 普通文本按次计费与原生模型列表

Rust Core 为普通文本调用提供显式按次销售价格，并为 Anthropic、Gemini 客户端提供当前可调用模型的只读列表。价格、身份、权限、预扣、终态回执和恢复均由 Core 处理；列表请求不调用供应商，也不创建预扣。

实现入口是 [文本价格合同](../services/core/src/model_http/text_request_price.rs)、[计费桥接](../services/core/src/model_http/billing.rs)、[恢复](../services/core/src/model_http/checkpoints.rs)和[原生列表](../services/core/src/model_http/native_models.rs)。本文描述实现合同；开发自测、独立验收、真实供应商兼容性和部署是不同结果。

## 启用与价格单位

沿用 [Core TOML](../config/core.example.toml) 的 `[model].config_file`、显式路由、私密凭据文件和 `external_models` 供应商合同。没有独立的“按次计费开关”：所选价格规则必须满足以下全部条件，才能冻结为 `text_request` 合同。

- `billing_mode = "tokens"`，没有 tiered 价格。
- `rates.request > 0`，单位为**每个逻辑请求的整数微美元**。
- `rates.input`、`output`、`cache_read`、`cache_write` 全为零。
- 实际选中的协议为 `openai_chat`、`openai_responses`、`anthropic_messages` 或 `gemini` 对应的普通文本转发合同。

现有 token 规则不变。所有费率为零仍表示原有免费 token 规则。请求费与非零 token 费的混合规则没有对应的普通文本计费合同，会被拒绝。Embeddings、Rerank、System One、图片及 DeepSeek Search/Fetch 的专用计费合同不会借用这个文本合同。

下面是 `core-model.toml` 中一条普通文本价格规则的片段，需与已有同名 `model`、`group`、`speed` 路由组合；不是一份可直接启动的完整配置。

```toml
[[prices.rules]]
id = 1
model = "text-public"
group = "allowed"
speed = "standard"
billing_mode = "tokens"
group_multiplier_ppm = 1000000
speed_multiplier_ppm = 1000000

[prices.rules.rates]
input = 0
output = 0
cache_read = 0
cache_write = 0
request = 750
```

`request = 750` 表示 USD 0.000750/次。分组和速度倍率均为 1 时，按固定 `500000 credits/USD` 换算，一次预扣和成功结算为 375 credits。用户信任折扣、分组倍率、速度倍率和汇率在同一请求中冻结，最后统一向上取整，不能对中间步骤分别取整。预扣金额为一次费用；冻结合同中的 input/output 上界仍用于供应商回执校验，不能因按次计费取消用量边界。

上游协议配置仍使用 `openai_chat`、`openai_responses`、`anthropic_messages`、`gemini`。例如，已有 HTTPS Chat 渠道对应的 `external_models` 条目必须匹配上游 ID、真实模型、`protocol = "openai_chat"` 和规范化 endpoint，并声明真实的 input/output 上界与 `contract_revision`。模型名称不决定供应商、价格单位或错误语义。

原生价格投影使用 `unit = "successful_requests"`、`request = 每次美元价格`，token 单价为空。原生日志另用更精确的 `billing_method = "per_completed_request"` 和 `completed_requests` 区分普通文本合同。钱包/订阅消费、预算、权限和余额检查沿用已有 Core 事务。

## 请求与费用结果

请求体和端点不变。客户端不传入价格、付款人或“成功次数”。例如：

```sh
curl https://YOUR_GATEWAY/v1/chat/completions \
  -H 'Authorization: Bearer YOUR_LMM_API_KEY' \
  -H 'Content-Type: application/json' \
  -H 'Idempotency-Key: text-operation-001' \
  -d '{"model":"text-public","messages":[{"role":"user","content":"Hello"}],"max_tokens":128}'
```

同一合同也适用于已经支持的 Responses、Anthropic Messages 和 Gemini GenerateContent 转发。流式调用仍需安装配置显式启用已有的 durable streaming 路径；按次规则本身不会打开流式功能。

| 可确认的结果 | 已完成次数 | 资金行为 |
| --- | ---: | --- |
| 供应商完整语义终态、实际 input/output 用量可解析且在冻结上界内 | 1 | 按冻结价格结算一次，真实 token/cache 计数继续保留 |
| 完整终态明确报告零 input/output tokens | 1 | 仍按一次请求收费，不将零 token 当作未执行 |
| 明确未发送，例如连接建立失败 | 0 | 保存 `NotSent` 证据，零费结算并释放预扣 |
| 经供应商合同确认的受理前拒绝，且完整纯 error envelope 已验证 | 0 | 保存 `Rejected`、状态码和响应摘要，零费结算 |
| 普通 HTTP 429/5xx，未配置或未满足明确受理前拒绝合同 | 未知 | 保留预扣，进入 `reconcile` |
| 缺失/非法/越界用量、无完整语义终态、流中断、可能已经受理的超时 | 未知 | 保留预扣，不补零、不重发 |

模型已经执行后输出的拒绝内容，可以是合法的完整回答；若已有有效终态与用量，就计一次。这里的零费 `Rejected` 专指供应商没有受理工作，不能只从 HTTP 状态、错误文案或模型回答推断。

可选 `external_models.pre_acceptance_rejection_statuses` 是安装者依据供应商合同审核的状态码列表，默认空。声明一个状态码仍不够：Core 必须读取并验证完整纯错误响应，排除生成、工具和用量证据。未审阅的供应商不应配置这一列表。重试和跨组 fallback 继续遵守[冻结候选与首次销售价格](../services/core/src/routing/README.md#model-http-retry-and-session-affinity)；多次确定未受理的尝试只属于一个逻辑请求。

## 幂等、恢复与账单

同一个 API Key 下重复使用相同 `Idempotency-Key` 不会再次发送上游；返回冲突供调用方读取原结果。并发重复也只有一个金融 start 能获得派发权。省略该头会创建新逻辑请求。

成功终态先保存供应商输出检查点；明确未受理的结果保存 Core `usage:provider-final` typed 回执。结算失败后，主机恢复任务读取原请求、原价格、原付款来源与原证据，通过已有 revision CAS 结算。它不会再次调用供应商，也不会用当前价格重新计费。没有权威完整回执的 UNKNOWN 请求继续冻结，恢复扫描不能把它改成零费。

`/api/log/self`、管理员原生日志、统计和导出都保存这一区别：

- `completed_requests = 1`：一次已完成文本请求，有真实 token 计数。
- `completed_requests = 0`：明确未受理。`token_usage_available = false`，不显示伪造的零 token。
- `completed_requests = null`：未完成结算，次数仍未知。
- CSV 使用 `completed_text_requests` 和 `billing_method`；未知 token 单元格为空。

新证据放在现有冻结价格 JSON 与用量事件 JSON 中，原有未包含 `text_request` 的快照继续按其旧合同读取。本文的两项能力不引入独立数据库迁移；同一软件版本的其他模块如需升级，仍以该版本的离线升级说明为准。

## Anthropic 模型列表

`GET /v1/models` 根据请求协议分派。存在 `x-api-key` 或 `anthropic-version` 时返回 Anthropic 结构；普通 `Authorization: Bearer` 请求保持原有 OpenAI `object/list` 结构。原生列表接受当前 Core API Key；Web session 和现有 model OAuth grant 不自动获得这一原生权限。

```sh
curl 'https://YOUR_GATEWAY/v1/models?limit=20' \
  -H 'x-api-key: YOUR_LMM_API_KEY' \
  -H 'anthropic-version: 2023-06-01'
```

列表读取支持版本头 `2023-06-01` 和 `2026-10-21`。这不扩大 Messages 请求解析器的版本范围。`limit` 默认 20、范围 1–1000。下一页使用 `after_id=上一页last_id`；上一页使用 `before_id=当前页first_id`；二者不能同时提供。未知或当前无权访问的 cursor 使用同一个无泄露错误。重复参数和不支持的参数会返回错误。

返回 `data`、`first_id`、`last_id`、`has_more`。空页的两个 ID 为 null。每个模型包含 `type="model"`、公开 ID、display name、受配置约束的 token 上界；供应商发布日期未知时 `created_at` 为 RFC 3339 epoch。没有供应商 capabilities 时保留 null，不从名字猜测能力或产品线。公开 ID 采用稳定字典序；本地没有供应商发布日期用于排序。当前不支持 `lifecycle` 筛选、beta 或 Workspace 选择头，也不提供 TypeSafe 的模型列表合同。

只有实际可执行的原生 Anthropic 路由才进入该列表：当前转发器要求 Anthropic upstream 的分区缓存回执，不会把任意 Chat 上游因为名称像 Claude 就列入。

## Gemini 模型列表与单模型

```sh
curl 'https://YOUR_GATEWAY/v1beta/models?pageSize=50' \
  -H 'x-goog-api-key: YOUR_LMM_API_KEY'

curl 'https://YOUR_GATEWAY/v1beta/models/text-public' \
  -H 'x-goog-api-key: YOUR_LMM_API_KEY'
```

也可使用 Bearer API Key；不能同时提供两种凭据。URL 中的 `key=` 不受支持。GET 请求体必须为空。

- `GET /v1beta/models`：`pageSize` 默认 50，要求正整数，上限按官方行为截到 1000；下一页带原来的 `pageSize` 和返回的 `pageToken`。
- 返回 `models` 和可选 `nextPageToken`；末页省略 token。
- token 绑定当前 Key、可见模型和上界、分组以及路由/价格版本。权限或配置改变会使旧 token 失效，客户端重新从第一页开始；token 不授予任何额外访问权。
- `GET /v1beta/models/{public-model}` 仅返回当前授权且可执行的模型；不存在和无权访问统一返回 `404 NOT_FOUND`。

每个模型使用公开 `models/{id}` 和 `baseModelId`，`supportedGenerationMethods` 包含实际支持的 `generateContent`。input/output 上界来自当前已验证的路线与安装上限。未配置供应商版本时 `version="unknown"`，描述明确说明是网关路由；不按模型名拆出虚构版本、温度默认值、thinking 或其他能力。

所有筛选都在分页之前完成，包括当前 Key 的模型、分组、来源地址权限、渠道可用状态、凭据和价格合同，以及真实转发路径的无 I/O 构造检查。分页、查详情和发现接口都不会访问收费上游。

## 定向开发自测

下面命令从仓库根目录执行；PG 用例仅使用 `DATABASE_URL` 指向的隔离测试数据库和生成的受限 runtime 角色。无付费上游请求。不要将生产 URL 用于测试。

```sh
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib model_http::text_request_price::tests
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib model_http::billing::text_request_usage_tests
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib model_http::text_request_tests
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib model_http::native_models::tests
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib model_http::catalog::tests
cargo +1.99.0 test --manifest-path services/core/Cargo.toml --locked --lib usage::export_tests
```

真实 HTTP→PG fixture 覆盖固定次数与真实 token、流式终态、并发幂等、明确拒绝、未知结果、NotSent、权限与余额阻断，以及改变当前价格后的原价恢复。列表 fixture 覆盖协议分派、权限先筛选、游标、单模型与 OpenAI 兼容形状。断言不是线上供应商或当前部署已经通过的证明。

## 协议来源

列表合同于 2026-10-30 核对 [Anthropic List Models](https://platform.claude.com/docs/en/api/models/list) 和 [Gemini Models](https://ai.google.dev/api/models)。本文明确列出的网关限制优先于供应商文档中尚未实现的可选字段或方法。

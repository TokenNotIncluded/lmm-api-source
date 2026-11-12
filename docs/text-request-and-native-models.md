# 模型调用、客户端与原生列表

Rust Core 为普通文本调用提供显式按次销售价格，并为 Anthropic、Gemini 客户端提供当前可调用模型的只读列表。价格、身份、权限、预扣、终态回执和恢复均由 Core 处理；列表请求不调用供应商，也不创建预扣。

实现入口是[文本价格合同](../services/core/src/billing/text_request_price.rs)、[计费桥接](../services/core/src/model_http/billing.rs)、[恢复](../services/core/src/model_http/checkpoints.rs)和[原生列表](../services/core/src/model_http/native_models.rs)。供应商 token 计价、输入预留和历史结算策略见[模型账单](model-billing.md)。本文另收录 System One 与客户端接入；标明历史的合同不代表当前 Core 已迁移，测试与部署结果仍以[项目进度](progress.md)为准。

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

新证据放在现有冻结价格 JSON 与用量事件 JSON 中，原有未包含 `text_request` 的快照继续按其旧合同读取。文本按次与原生列表不引入独立数据库迁移；同一软件版本的其他模块如需升级，仍以该版本的离线升级说明为准。

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

## System One

TypeSafe Jev 的同步判断由 [Core 原生入口](../services/core/src/model_http/systemone.rs)处理，不经过 Chat 转换器或 JavaScript 插件宿主。`POST /v1/systemone` 与 `POST /typesafe/v1/systemone` 接受 Bearer Core API Key，并检查模型、分组和来源限制；Web session、现有 Pi/model OAuth grant 在预扣前被拒绝，不支持流式。

配置使用 `protocol = "system_one"`、私密凭据、已启用的 group/route/target、上游模型映射与审核价格。供应商基址为 `https://api.typesafe.ai`，可带 `/v1`；拼接 `systemone` 时保留路径前缀。客户端不能指定上游 URL、供应商密钥、付款方或价格。Web 渠道类型仍为 62，OpenHuman 61 不变；目录 API 名 `system-one`、Web endpoint 名 `systemone` 与路由值 `system_one` 要显式映射。

Web 候选为 `jev-1.14.0`、`jev-1.13.0`、`jev-latest`、`jev-preview`。2026-10-21 核对的[官方模型页](https://docs.typesafe.ai/models)将 latest 指向 1.13.0、preview 指向 1.14.0；这是当时的别名记录，不能代替当前供应商核验。仅返回别名的模型列表不应删掉版本化 ID。

```sh
curl https://YOUR_GATEWAY/v1/systemone \
  -H 'Authorization: Bearer YOUR_LMM_API_KEY' \
  -H 'Content-Type: application/json' \
  -d '{"model":"jev-1.14.0","state":{"message":"Reset a forgotten password"},"questions":{"account_support":{"type":"noul","instructions":"Is this an account support request?"}}}'
```

SDK decision 方法使用 `https://YOUR_GATEWAY` 或 `https://YOUR_GATEWAY/typesafe`，显式传模型。网关没有 TypeSafe SDK `models.list()` 合同；普通 `/v1/models` 仍返回 OpenAI 结构。

| 请求边界 | 合同 |
| --- | --- |
| 结构化输入 | 保留 `state`、命名 `questions`、可选 `assets`；问题为 `noul/choice/score/box`，choice 的 criteria 是对象，score 是有序数组，box 不带 criteria |
| 回答校验 | 类型、概率、score legend、非零 Box 几何须匹配；JSON 数字保留精度及普通用户键，拒绝重复键与非有限值 |
| 图片授权 | 每次请求设置 `TypeSafe-Danger-Zone-Images: true`；Box 还要求 Jev 1.14（显式版本或对应 preview）和实际图片 |
| 图片载荷 | PNG/JPEG 的内联 `data_base64` 或 multipart 引用；首个 part 为 `application/json` 的 `request`，唯一图片 part 与 assets 一一对应；引用可在嵌套字符串值中，不解释对象键 |
| 大小与解码 | wire 与解压后各最多 32 MiB，单张解码/上传图片最多 16 MiB；本地检查文件头、280–1568 尺寸和非动画 PNG |
| 压缩 | 只解一层 `gzip/x-gzip/zstd`，zstd window 最多 8 MiB；错误、叠加或超限编码拒绝，覆盖整个 multipart 的压缩；解压与解析共用最多两个请求的 gate |
| 长上下文 | 另需 `TypeSafe-Danger-Zone-Long-Context: true` 和安装方审核的更大上界；请求头不能提高安装上界，不由官方缩写 `262k` 猜测精确值 |

完整图片解码、供应商重新编码及展开引用后的限额仍由供应商检查，原始大小乘引用次数不能证明处理后大小；见[官方图片合同](https://docs.typesafe.ai/images)。对应实现为[请求合同](../services/core/src/model_http/systemone_contract.rs)、[multipart](../services/core/src/model_http/systemone_multipart.rs)与[压缩边界](../services/core/src/model_http/systemone_compression.rs)。

计费只接受冻结的标量输入 token 价格；输出、折扣缓存和按次费率为零，不恢复历史表达式价格。输入预留上界至少 65,536 tokens，外部合同须匹配协议、endpoint、model、revision，并显式设 `missing_cached_tokens_is_zero = true`。完整整数输入回执须在上界内；先保存终态、结算，再返回答案。非零输出用量仍不计输出费，明确零输入与缺失用量不同。不完整或不确定工作保留预扣；幂等、断连后记录、drain 与 `Cache-Control: no-store` 沿用原生合同。

2026-10-21 记录的供应商价格为 USD 0.042/百万输入 token、输出免费，图片计入输入；它不改写客户已保存价格。独立 SDK、真实供应商和部署验收仍需各自证据，访问/转售条件见 [TypeSafe MCA](https://typesafe.ai/legal/mca)。

来源：[System One API](https://docs.typesafe.ai/api/system-one)；2026-10-20 OpenAPI SHA-256 为 `ee599512cbea3a198b34af3f60addcd72b381d3521196f93c70ae83185c057c3`。审核过的 MIT SDK 为 JavaScript 0.7.0（`b844cd1aca12855a5d24a2df52a1401c86e7d37a`）及 Python 0.7.5（`f36e68549ceca3191e5791e5f99ea1baa1a258b5`）。历史 Go 行为来自 Apache-2.0 的 `QuantumNous/new-api-plugins` TypeSafe 1.0.0（`97a16e9a8d98b73ffb76e1c5a00b4d0f855f4d0a`），保留归属，未导入可执行插件或任务宿主。

## 客户端接入

下列适配器由[子模块](../.gitmodules)固定独立仓库版本，源码、包、CI 与宿主测试不在父仓库重复维护。它们的原接入合同使用公开 native OAuth client、PKCE S256、state/issuer 绑定的 loopback callback；同意范围为 `catalog:read balance:read usage:read models:invoke` 加用户明确选择的账户组，不自动授予 MCP、市场或管理权限。旧客户端登记和测试记录不证明当前 Core 的部署开关、授权登记或付费互操作已通过。

### Codewhale

[`packages/codewhale-lmm-provider`](https://github.com/TokenNotIncluded/codewhale-lmm-provider) 对应原客户端 `lmm-codewhale` / `LMM for Codewhale`，沿用 resource、refresh rotation 与撤销合同，不改变 Pi/DSH/CLI grant。

原接入使用 companion CLI 和命名 `openai-compatible` provider：当时 Codewhale bundle API 没有可执行 provider/auth adapter，bundle 仅含帮助 skill。只接纳目录中声明 `openai-completions` 的模型，不声明 Responses/Messages-only、Pi picker、MCP 或市场能力。必须以 `codewhale-lmm run` 启动；CLI 持有 LMM 凭据，宿主仅通过 `api_key_env` 获得每次运行随机的 loopback capability，临时配置不覆盖宿主配置。

桥接每次请求重查目录和 grant，将精确 synthetic model ID 换成上游模型并设置 `X-LMM-Group`，不 fallback；SSE 原样传递，断连取消上游，不重试推理 POST，宿主自己的重试策略另计。历史本地证据为 Linux / Node.js 22.16.0 的 36 项测试及语法/打包检查，包含 mock OAuth HTTP、跨进程刷新锁和 mock 宿主；不是 live Codewhale 或账单验收。旧布局命令 `cd apps/lmm-extensions && go test ./oauthserver -run TestCodewhale -count=1` 对应初始 CI `35638700004`，当时本地依赖不可用未跑；当前 Extensions 已无该 `oauthserver` 路径。

### OpenCode

[`packages/opencode-lmm-auth`](https://github.com/TokenNotIncluded/opencode-lmm-auth) 对应原客户端 `lmm-opencode`。安装器合同要求 Node.js ≥22.19，使用官方最新宿主及最新已发布、校验和验证的插件；1.18.34 仅是历史回归基线，不是安装锁定版本。从[公开脚本页](https://api.lmm.best/scripts)的 OpenCode 入口或 `packages/lmm-scripts/opencode.sh` / `opencode.ps1` 安装，保留其他配置/插件，修改前备份；重跑更新受管理插件项，不自动登录。

执行 `opencode auth login --provider lmm`，选择 **Sign in with LMM (OAuth)** 并完成浏览器同意，再启动/重启宿主。CLI 是首次登录入口，未连接的 provider 不在 TUI picker 暴露模型。插件使用宿主凭据库和本地 refresh replay journal；目录中的 Chat、Responses、Messages 分别使用对应 SDK 和精确账户组，不支持的能力/计费单位不列出。凭据只发往配置的 HTTPS issuer，拒绝重定向，调用按账户正常计价。

脚本仓库合并后须在 **运维 → 脚本 → 拉取更新** 发布 root `.sh/.ps1`；共享安装器从固定 GitHub revision 下载。只改本地 gitlink 不会更新公开脚本页。独立包检查入口：

```sh
git submodule update --init -- packages/opencode-lmm-auth
cd packages/opencode-lmm-auth
npm ci --ignore-scripts
npm run build
npm run typecheck
npm test
npm run pack:check
# 后两项需要受支持的官方 OpenCode 宿主及 OpenSSL
npm run test:host
npm run test:integration
```

integration fixture 在真实宿主中检查 OAuth 和流式响应；生产登录与付费推理不由 fixture 成功推定。

## Ollama 历史传输

以下是旧单体及当时 Rust relay 的行为记录，不是当前 Core 的渠道迁移承诺。Web 表单仍保存 type `4` 的 `settings.ollama_openai_chat` 布尔值（默认 false），与 `setting`、Advanced Custom routes 分开；可在[表单转换](../apps/web/src/features/channels/lib/channel-form.ts)核对字段保留。

| 旧请求 | flag 关闭 | flag 开启 |
| --- | --- | --- |
| 普通 Chat / Chat playground | `/api/chat` | `/v1/chat/completions` |
| Completions / Embeddings | `/api/generate` / `/api/embed` | 不变 |
| Responses / Messages body passthrough | `/v1/responses` / `/v1/messages` | 不变 |
| Messages 非 passthrough | 转换到 `/api/chat` | 不变 |

旧 Go 开启后使用 OpenAI 请求转换与 JSON/SSE 处理器；转换后的流请求上游 usage，但保留客户端隐藏下游 usage 的选择，body passthrough 原样。发现、pull、delete 仍走原生管理 API。旧 Rust 只实现显式 OpenAI Chat 和独立 Responses；native chat/generate 在预扣和 I/O 前报 HTTP 501 `ollama_native_transport_unsupported`。这条旧 Go 使用建议不适用于当前仅由 Core 承接模型流量的架构。

## Responses 历史工具转换

旧客户端配置使用 `wire_api = "responses"` 和站点 `/v1` 基址，依据[Codex 配置](https://learn.chatgpt.com/docs/config-file/config-reference)、[工具搜索](https://developers.openai.com/api/docs/guides/tools-tool-search)与[命名空间函数](https://developers.openai.com/api/docs/guides/function-calling)。以下表格记录旧 Go 转换注册器能力；当前 Core 仍以[请求解析器](../services/core/src/relay/request.rs)和实际路由为准。

原生 Responses 渠道保留 `tools/input/tool_choice` 与 JSON/SSE 工具数据，仍要求供应商支持。仅 Chat 上游曾通过 Advanced Custom 入站 `/v1/responses` → 上游 `/v1/chat/completions` 配置及 `openai_responses_to_openai_chat_completions` 转换器接入，认证沿用渠道；不因上游报错自动换协议。旧 Chat/Gemini 转换行为如下：

| 结构 | 历史行为 |
| --- | --- |
| namespace 函数 | 分配合法、不冲突的上游名；历史调用和 tool_choice 共用映射，JSON/SSE/最终响应恢复 namespace 与 name |
| 默认或 server tool_search | 提前加载请求及已返回搜索结果的函数，移除搜索声明与完成的服务端搜索历史；可能增加上下文 |
| client tool_search | 向上游表现为普通函数，向客户端恢复 tool_search_call 和对象 arguments；客户端实际执行 |
| tool_search_output | 转为对应工具结果，并在下一轮加入动态工具 |

映射仅在单请求 `convmeta.Meta` 中复用，覆盖请求、响应与流结束；独立 relaykit 可用 `convmeta.Values`，不改客户端原请求、不跨用户共享。网关不执行客户端搜索或业务工具，也不据模型返回访问外部服务。提前加载必须有工具定义，不实现供应商专用搜索引擎；强制 server search 等不能等价转换的请求拒绝。

自定义/托管工具及状态字段仍受原转换器限制；该历史变更没有新增 Claude/AWS/Vertex 原生 Responses 或 Rust 预览后端能力。原回归使用本地协议数据和模拟上游，线上客户端及供应商兼容性仍需单独证据。

## 定向开发自测

下面命令从仓库根目录执行；PG 用例仅使用 `DATABASE_URL` 指向的隔离测试数据库和生成的受限 runtime 角色。无付费上游请求。不要将生产 URL 用于测试。

```sh
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib billing::text_request_price::tests
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::billing::text_request_usage_tests
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::text_request_tests
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::native_models::tests
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib model_http::catalog::tests
mise exec -- cargo test --manifest-path services/core/Cargo.toml --locked --lib usage::export_tests
```

真实 HTTP→PG fixture 覆盖固定次数与真实 token、流式终态、并发幂等、明确拒绝、未知结果、NotSent、权限与余额阻断，以及改变当前价格后的原价恢复。列表 fixture 覆盖协议分派、权限先筛选、游标、单模型与 OpenAI 兼容形状；System One 的本地供应商/PG 入口见[集成测试](../services/core/tests/systemone_independent.rs)。测试源码和历史通过记录不代表本轮已执行或线上部署已验收。

## 协议来源

列表合同于 2026-10-30 核对 [Anthropic List Models](https://platform.claude.com/docs/en/api/models/list) 和 [Gemini Models](https://ai.google.dev/api/models)。本文明确列出的网关限制优先于供应商文档中尚未实现的可选字段或方法。

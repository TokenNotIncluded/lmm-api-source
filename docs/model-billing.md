# 供应商计费与原生用量

本文说明模型配置、输入预留、供应商回执与原生用量查询；末尾保留旧结算策略，不能据此推定当前 Core 已迁移。
当前工作项见[项目进度](progress.md)，历史交付和验收记录见[归档](archive.md)。

## 接口与配置

模型数据面由 Core 负责。主配置的 `[model].config_file` 指向安装方的模型 TOML；
外部供应商使用 HTTPS，`local_http = false`。完整路由结构见[路由指南](../services/core/src/routing/README.md)，
字段以[配置加载器](../services/core/src/config.rs)和[路由配置](../services/core/src/routing/config.rs)为准。

| 操作 | HTTP 入口 | 供应商协议与计价边界 |
| --- | --- | --- |
| DeepSeek 聊天 | `POST /v1/chat/completions` | `openai_chat`；分别计普通输入、缓存读取和输出 |
| Search / Fetch | `POST /v1/tools/search`、`POST /v1/tools/fetch` | `deepseek_search` / `deepseek_fetch`；仅成功请求费，token 单价全为零 |
| Embeddings | `POST /v1/embeddings` | `openai_embeddings`；仅输入 token 价格，输入上界覆盖整个 batch |
| Jina Rerank | `POST /v1/rerank` | `jina_rerank`；仅输入 token 价格，上界覆盖 query 和全部候选文档 |
| Gemini Google Search | `POST /v1beta/models/{model}:generateContent` | `gemini`；token 费加最多一次 grounding 费，`rates.request` 与 `rates.cache_write` 为零 |

Search/Fetch、Embeddings 和 Rerank 使用模型 API Key；生成接口的 OAuth scope
不会自动授予这些入口的调用权。Search/Fetch 不从请求体接受模型、目标或价格。
Search 接受最长 128 字符的 `query`、1–50 的 `count`，以及 `ori_question`、
IP 字符串 `localization` 和布尔 `need_content`；Fetch 接受最长 8192 字符、
有主机且不含账号密码的 HTTP(S) URL，由已配置的供应商抓取。

每条 `external_models` 合同的 `upstream_id/upstream_model/protocol/endpoint`
必须与实际目标一致，输入上界须覆盖整次请求。目标使用 `payload = "adapt"`，
`timeout_ms` 不小于全局 `timeout_seconds × 1000`。供应商密钥放在独立私密文件，
配置中的 `contract_revision`、上下文上界与费率来自安装方审核的供应商合同。

## 单位与价格冻结

| 字段 | 单位 |
| --- | --- |
| `prices.rules[].rates.input/output/cache_read/cache_write` | 每一百万 token 的 micro-USD；`1000000` 表示 USD 1 / 百万 token |
| `prices.rules[].rates.request` | 每次逻辑请求的 micro-USD；Search/Fetch 按成功请求计数 |
| `external_models[].grounding.usd_micros_per_grounded_request` | 每次 grounded request 的 micro-USD |
| `group_multiplier_ppm`、`speed_multiplier_ppm` | 百万分比；`1000000` 为 1 倍 |
| Core credits | USD 1 = 500000 credits，最终金额向上取整到整数 credit |

Core 在预留前冻结授权分组、路由目标、凭据 revision、供应商合同与价格版本。
结算和恢复使用原报价，后续改价不改变已接收请求的账单。未知回执保留原预留；
客户端断开不会取消账务责任，相同 `Idempotency-Key` 不会重新派发相同逻辑请求。

Search/Fetch 使用标量 `billing_mode = "tokens"` 价格结构，但所有 token 单价必须
为零，仅 `rates.request` 可非零；不接受 `tiered_decimal` 或 `upstream_cost`。
例如审核后的单次价格若为 750 micro-USD，在无折扣且倍率均为 1 时计 375 credits；
此数值仅说明单位换算，不是供应商报价。普通文本按次计费另见[专门指南](text-request-and-native-models.md)。

Gemini grounding 需在对应 `[[external_models]]` 内显式配置：

```toml
[external_models.grounding]
mode = "grounded_request_v1"
tool_input = "exclude_search_retrieval_v1"
usd_micros_per_grounded_request = 35000 # 单位示例，须替换为审核后的实际费率
```

这个合同只表达每次请求的 0/1 grounding 费，不能表示按搜索 query 数收费的合同。
调用使用 `tools: [{"googleSearch": {}}]`，不与函数声明等工具混用。
流式入口为 `:streamGenerateContent?alt=sse`，采用相同的价格冻结和计数规则。

## 输入预留与精确计数

原生 Responses compact 与加密上下文重放见[压缩上下文](responses-compaction.md)。
它们使用显式审核的完整上下文上界，不属于下述纯文本精确计数 profile。

`external_models[].reservation` 是[严格 tagged union](../services/core/src/model_http/provider_policy.rs)。省略或 `mode = "context_max"` 使用审核后的完整输入上界，不请求计数器；未知 mode/字段、缺失或为零的 exact timeout 均为配置错误。

`openai_responses_exact_v1` 只接受显式审核的官方 `https://api.openai.com/v1` 基址、`openai_responses` 协议和固定日期模型。日期后缀仅排除 alias，不证明模型存在或供应商计数语义正确。以下仅示范字段，限额和 revision 必须按实际合同替换；原记录中的真实供应商验收仍为 **NOT_RUN**。

```toml
[[external_models]]
upstream_id = 1
upstream_model = "gpt-4.1-2025-04-14"
protocol = "openai_responses"
endpoint = "https://api.openai.com/v1"
max_input_tokens = 1048576
max_output_tokens = 32768
contract_revision = "replace-with-reviewed-contract-revision"

[external_models.reservation]
mode = "openai_responses_exact_v1"
count_timeout_ms = 2000
```

该 profile 仅支持无状态、非流式纯文本。最终编码体必须含 `model/input/store:false/stream:false` 及正的 `max_output_tokens`；可选 instructions 为字符串。input 可为字符串或完整的 system/developer/user/assistant 消息，文本块用 input_text，assistant 也可用 output_text。计数投影保留全部消息和 instructions。tools、历史 item 引用、conversation ID、媒体、schema、cache controls 及未审核 sampling/reasoning 字段均拒绝；准入应在编码器可能丢字段前完成，不通过删字段、换路由或降级 profile 接受请求。

路由和编码冻结后，[计数实现](../services/core/src/relay/input_count.rs)把完整 model/input/instructions 投影发到固定 `https://api.openai.com/v1/responses/input_tokens`，与生成使用同一凭据和 ProviderTransport。DNS/TLS、origin/peer 固定、禁代理/重定向以及 connect/header/read/body 限额一致；gate 总 deadline/cancellation 与专用 count timeout 共同覆盖交换。连接准备前后都检查 route availability，撤销后不能继续计数。

响应只接受 `object = "response.input_tokens"` 和不超过原模型输入上界的无符号整数。不估算、不截断、不换路由重试、不回落 context-max；失败发生在预留前，不产生 usage/start 派发许可。观察记录仅保存计数、输出 cap、投影版本及冻结生成体/计数请求/响应的 SHA-256，不含正文、instructions、API Key 或 Authorization。它是准入证据，不是 usage 或结算回执；生成必须发送同一份保留字节，金融合同准备前再次核对路由与编码体。

### 准入与恢复

派发前，[冻结合同](../services/core/src/billing/charging/contract.rs)将 `reservation-contract:v1` 与 `reservation-admission:v1` 在请求、预算检查和钱包预扣的同一 SERIALIZABLE 事务内保存，固定 route、凭据引用/revision、wire digest、输入/输出上界、缓存含义与价格。两者都不能带可结算 usage，audit-only provider observation 也不能代替终态。

配对证据缺失、格式错误或不匹配会阻止派发/结算，恢复保留原预扣，不为旧 UNKNOWN 补证、释放或记零。旧负债与另行补偿分开；恢复使用存储合同，预留 COMMIT 确认丢失用原 request ID 核实，派发确认丢失不授权第二次发送。

[System One](text-request-and-native-models.md#system-one) 使用独立 `systemone_native_v1`：保留完整输入上界和输入单价，绑定有界 JSON/multipart 字节，output 仅为不计价元数据，不声明生成输出上界或调用 Responses counter；通用恢复仍检查原配对证据。

该合同使用既有事件存储，不单独引入 schema migration；旧 recovery worker 必须理解合同版本和语义才能处理新请求，不能据此承诺二进制回滚兼容。[默认 Billing::reserve_prepared](../services/core/src/relay/types.rs) 只接受无 count observation 的 context-max；exact route 必须显式实现 prepared adapter，不能静默调用旧 reserve。

定向 parser/adapter 检查如下。本地 counter fixture 只验证代码，不证明官方供应商、生产配置或部署验收。

```sh
mise exec -- cargo test --manifest-path services/core/Cargo.toml --lib input_count
```

## 回执与计费依据

**缓存和推理用量保留供应商明确计数。** DeepSeek 的 `prompt_cache_hit_tokens`
映射为缓存读；与 `prompt_tokens_details.cached_tokens` 同时出现时必须一致。
若 hit、miss 和总输入同时存在，须满足 `hit + miss = prompt_tokens`。
缺失缓存计数保持未知，只有合同显式配置 `missing_cached_tokens_is_zero` 或
`missing_cache_creation_tokens_is_zero` 才采用对应的缺失即零规则。
`reasoning_tokens` 是输出总量的子集，不重复相加。Chat 的 `reasoning_effort` 和
Responses 的 `reasoning.effort` 接受 `none/minimal/low/medium/high/xhigh/max`，
按原值转发；端点是否接受该档位仍由选中的供应商合同决定。

**Search/Fetch 根据完整回执确定成功次数。** Search 的 HTTP 200 JSON 必须有
`results` 对象数组，非空计 1、空数组计 0；Fetch 必须有字符串 `content`，
去空白后非空计 1、否则计 0。通过传输校验的完整 4xx/5xx 回执计 0，操作结果为
`failed`；空结果的 HTTP 200 为 `completed`。缺失字段、错误结构、超时或不完整
响应保持待核对，恢复读取原 HTTP 状态、计数与摘要，不把未知结果当成免费成功。

**Grounding 计数与检索 token 分开保留。** 非空 `webSearchQueries` 或带有效 URI
的 web grounding chunk 建立发生 grounding 的证据；多块流只累计同一请求的 0/1 状态。
明确空 query 且无 web grounding 可证明 0，完全缺失证据仍为未知。
`usageMetadata.toolUsePromptTokenCount` 保留为 `tool_input_tokens`；上述合同不将
其再加入可计费输入。Grounding 与 token 费用合并后应用冻结倍率和折扣，只做一次 credit 取整。

**Embeddings/Rerank 要求完整输入用量。** Embeddings 接受字符串、字符串数组、
token ID 数组及其批量形式，最多 2048 项，支持 `float/base64` 与可选 `dimensions`。
响应必须覆盖全部索引、维度一致且数值有限，`usage.prompt_tokens == usage.total_tokens`。
Rerank 接受非空 `query`、最多 2048 个字符串 `documents` 和合法的
`top_n/return_documents`；使用 `usage.total_tokens`，同时出现的 `prompt_tokens`
须一致。两者输出计费用量固定为 0，不按向量维度或结果数量推算 token。

实现入口：[缓存解码](../services/core/src/relay/decode.rs)、[工具价格与回执](../services/core/src/billing/tool_price.rs)、
[grounding 合同](../services/core/src/billing/grounding_price.rs)、[输入型模型校验](../services/core/src/model_http/input_contract.rs)。

## 原生用量与导出

| 身份与用途 | GET 入口 |
| --- | --- |
| 管理员列表、汇总 | `/core/v1/usage/admin/logs`、`/core/v1/usage/admin/stats` |
| 管理员／个人 CSV | `/core/v1/usage/admin/export.csv`、`/core/v1/usage/self/export.csv` |
| 个人列表、汇总 | `/api/log/self`、`/api/log/self/stat` |
| 管理员／个人看板 | `/api/data`、`/api/data/self` |

这些入口使用 Web 会话；管理员入口另查管理权限。个人范围按操作者与个人密钥
所属账户限定，团队出资的个人密钥记录仍在个人范围，团队所属密钥不因此暴露。
历史日志与原生用量是不同来源，页面切换来源后使用对应查询和导出接口。

共同筛选为 `start_timestamp/end_timestamp`（Unix 秒）、`model_name`、`token_name`、
`group`、`request_id`；管理员可加 `username`。时间窗最多 31 天，缺省最近 31 天。
列表使用 `p/page_size`，每页最多 100；CSV 不接受分页参数，读取同一授权快照中的
完整匹配结果，输出最多 32 MiB。非零 `channel` 和非空 `upstream_request_id` 被拒绝。
超限、超时或证据损坏明确报错，不产生静默截断的成功文件。

账单区分预留、已结算、退款和净消费；未结算记录的最终用量与消费留空。
工具日志的 `tool_requests` 与 CSV 的 `successful_tool_requests` 表示同一 0/1 计数，
`outcome/provider_http_status` 区分操作结果与账务状态。Grounding 单列
`grounded_requests/tool_input_tokens`。缓存统计保留读、写与 5m/1h/12h 写入分项；
缺少完整证据为 `null`，显示 `—`，不能用零补齐。CSV 采用字段白名单并处理公式前缀，
不包含供应商原始响应、凭据或任意诊断对象。

查询与导出合同见[用量实现](../services/core/src/usage.rs)和[CSV 编码](../services/core/src/usage/export.rs)；
相关回归见[用量测试](../services/core/src/usage/index_tests.rs)、[输入接口测试](../services/core/src/model_http/input_api_tests.rs)
及[工具接口测试](../services/core/src/model_http/tool_api_tests.rs)。测试源码仅说明检查入口，执行结果以对应冻结记录为准。

## 历史结算策略

以下保留旧 Go 单体及当时 Rust 实现的政策和回归范围。当前 Core 的[供应商回执校验](../services/core/src/model_http/provider_policy.rs)与[结算接口](../services/core/src/relay/types.rs)要求原合同下的完整用量；缺失证据进入待核对，不能把历史估算或前端遗留设置直接当作新合同。

### Responses 历史缺失用量

旧实现将 text、refusal、function arguments、reasoning text/summary 的 delta 计为本地输出下界；仅在没有任何 delta 文本时，成功 `response.completed/response.done` 的终态 snapshot 才提供文本、拒绝、字符串函数参数及可见 reasoning。排除加密 reasoning、图片和工具 metadata，不改供应商原响应，也不猜补部分已观察 item 之外的其他 snapshot item。

已接受的供应商 usage 优先。生命周期零占位不能覆盖先前用量或压掉后来输出；终态明确零则抑制本地输出、历史/预扣 fallback，零与缺失必须分开。独立定价的实际工具调用仍按原规则。

| 历史证据 | 当时结算策略 |
| --- | --- |
| 无事件；created-only 后 EOF；无 usage/输出的 incomplete、cancelled、failed 或 flat error | 不估算消费，受理本身不证明输入已消费 |
| 无 usage 的成功终态生成内容，或已观察到可计费 delta | 使用输出下界和有界请求侧输入估算 |
| 实际输出后 EOF、读失败、客户端取消或 failed/error | 保留原部分结算与禁止重放行为 |
| 供应商终态明确零 | token 零费，不用历史/预扣 fallback |

输入估算上限为 1,050,000 tokens；成功、缺失 usage 且无可计数输出仍使用当时的 successful-empty fallback。flat error 立即结束，后续 completion 不覆盖失败，也不再发第二个 synthetic interruption；表面 completed 但带非 null error 不能产生终态输出估算。

[#373](https://github.com/TokenNotIncluded/lmm-api/issues/373) 曾要求显式失败且无 usage 时零 fallback，同时要求 [#341](https://github.com/TokenNotIncluded/lmm-api/pull/341) 的 `failed_text` 保持收费，两者冲突。维护者于 **2026-10-03** 明确选择保留已输出内容的部分计费；无输出失败、created-only、空 incomplete 继续为零。这是历史政策决定，不是本次新增结算规则。

原 Go HTTP/gzip 测试比较 wallet、Token、用户用量、消费日志 quota 和一次上游调用；覆盖拒绝中断、终态各类输出、snapshot 去重、明确零、completed-with-error、flat error，另有非流式零用量。历史 WebSocket 的 58 个上下游/SQL 用例还覆盖占位、五类 delta、失败部分输出、nested error、重放、新尝试和输入估算；下游成功槽与 token 结算独立。当时 Rust 有对应单测和显式 ignored 的 PostgreSQL HTTP 结算套件；普通测试通过不等于隔离 PG 套件运行通过，这些数量不是当前分支验收结果。

### Claude 历史无输出拒绝

旧 Go 的 **系统设置 → 模型 → Claude → Do Not Charge for Claude Refusals Without Output** 保存 `claude.refusal_no_output_no_charge_enabled`，默认/缺失均为 false。只应在所有选中供应商都遵守无输出拒绝免费合同时启用；它不改变第三方账单，混合或未审核供应商保持关闭。

豁免要求明确 Claude refusal、没有任何输出 content block、供应商明确报告 output tokens 为零；流式必须以最终 message_delta 为证，message_start 零占位不够。缺失 usage、单纯空响应、已有输出后的 refusal 均不满足；即使空 text/tool block 也阻止豁免。非空 `usage.iterations` 表示此前 fallback 可能收费，继续正常结算；畸形 category 保守处理。

按原文 2026-09 的[Anthropic 拒绝计费规则](https://platform.claude.com/docs/en/build-with-claude/refusals-and-fallback#how-refusals-are-billed)，`stop_details.category` 为 `bio/frontier_llm/reasoning_extraction` 的零输出拒绝仍收费，其他及 null/缺失 category 才可能豁免。这是历史核对时间，不证明供应商规则至今不变。

符合条件时退还 wallet、subscription、API-key quota 的全部预付，跳过倍率、固定价、tiered expression 与工具附加费；请求数增加，quota/token 消费汇总为零。消费日志保留真实 input/cache 诊断计数并标记 `billing_exempt_reason=claude_refusal_no_output`，refusal debug reason 仅在管理员 metadata，客户端 usage 原样。原 native Messages、转 Chat/Responses 共用证据校验；当时 Claude adaptor 接通已有 Responses 转换器，身份、组倍率、价格锁、`/fast`、OAuth 模式或 admin AI 状态都不是豁免依据。

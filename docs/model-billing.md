# 供应商计费与原生用量

本文说明模型配置、供应商用量如何进入账单，以及原生用量的查询接口。
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

# Responses 原生压缩上下文

`POST /v1/responses/compact` 使用当前 API Key、模型/IP/组权限、固定渠道与
Core 资金协议。请求包含 `model`、`input` 和可选字符串 `instructions`，返回
供应商的 `response.compaction` JSON，包含可继续使用的加密上下文项。
压缩接口本身不提供流式响应。

把返回的 `output` 项完整放入下一次 `POST /v1/responses` 的 `input`，再追加
本轮消息。该原生路径支持 JSON 和 SSE，以及函数工具、reasoning、text/schema
控制；保持 `store:false`。加密项不得先转换成文字或其他供应商的消息格式。
供应商 response ID 保留在这条原生路径的响应中。

## 配置与费用

现有 `external_models` 条目必须显式设置 `responses_compaction = true`，
并以审核过的供应商合同声明整个请求的 `max_input_tokens`、
`max_output_tokens`、模型、endpoint 和 `contract_revision`。
该标记属于模型条目本身，不属于 `reservation` 子表；默认关闭，旧合同中
未配置该标记时保留原有行为。

这条路径只支持原生 `openai_responses`、adapt payload 和普通 token 价格。
预留模式必须是 `context_max`；精确纯文本计数 profile 不接受加密上下文，
不得在失败后静默改用另一种预留方式。压缩和加密上下文重放按照审核后的完整
输入/输出上界预留，不能用密文字符数估算语义 token 数。预留可能较大，余额
不足时会在派发前拒绝；最终费用以可信实际用量和原冻结价格结算。

函数工具由调用方执行；这条路径不执行收费的托管搜索/抓取等工具。媒体、
无法表达的输入、未支持的价格维度或协议会明确拒绝，不会删字段后重试。

## 输出、失败与恢复

SSE 数据包先编码一次、记录实际发送字节的 SHA-256/长度，再发送同一份字节。
成功终值会暂存，继续有界读取到 HTTP body EOF，并验证解析器完整结束。
只有可信终态用量、检查点和资金结算得到确认后，才发送成功终值。
终值后的异常字节、重复终值、截断、超时或不明确的提交结果均保留
hold/UNKNOWN，不能补零结算。

客户端重试应保留原 `Idempotency-Key`。同一请求不会再次派发供应商；
重复调用可能返回冲突，不承诺重放原响应正文。恢复使用原请求、付款来源、
价格和证据，不重新执行推理，不重新计费。此功能不新增响应正文数据库。

## 当前边界

本入口不补齐通用 HTTP `previous_response_id` 的渠道亲和或 `store` 转交。
本地响应 GET/DELETE、TTL 和默认 `store:true` 不是这里的功能。
OAuth compact、跨供应商加密项迁移、媒体、托管收费工具、compact 的可选
cache/service-tier 控制及全部供应商等价性仍需各自实现和验证。

本地受控供应商验证与真实供应商验收、生产准入分别记录，不能互相抵扣。
资金与预留规则另见[模型计费](model-billing.md)。

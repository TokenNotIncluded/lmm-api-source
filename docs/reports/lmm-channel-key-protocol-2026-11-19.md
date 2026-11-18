# 渠道配置与 API Key 多协议迭代

日期：2026-11-19（Asia/Taipei）。

## 本轮结果

渠道页面改进已在新架构环境发布，运行版本为 [9f1bd154](https://github.com/TokenNotIncluded/lmm-api-source/commit/9f1bd154139f154b9e5952c4a9c8922a3e10c50f)。两台既有目标的健康检查、实际返回 HTML 与固定构建产物的匹配检查均通过。[查看渠道页](https://native.api.lmm.best/channels)。

管理 API 创建 Key、四种协议调用、逐笔结算与撤销后的拒绝检查已在真实 Core 路由和隔离 PostgreSQL 上通过。测试代码为 [97c50253](https://github.com/TokenNotIncluded/lmm-api-source/commit/97c50253122b8eb305518882d692fab9ae56c2e6)。供应商响应由本机合成服务提供；本轮没有执行登录态线上模型调用或真实供应商付费请求。

本报告记录开发自测和 Web 发布结果。全库 CI 仍有既有失败，完整线上业务验收尚未完成。

## 渠道配置改进

- 能力信息过期或读取失败时，编辑器继续保留已输入的草稿和模型选择。能力恢复前阻止写入，刷新成功后可继续操作。
- 模型选择支持搜索和多选添加，筛选条件改变时保留已选择模型。
- 模型合同、路由和供应商参数可折叠，减少表单同时展示的字段。
- 渠道列表提供启用／禁用及 API Keys 操作入口。批量编辑、密钥管理和余额配置也使用相同的能力失效保护。
- 四个新增翻译键已同步至七种语言，缺失键及多余键检查均为零。

编辑器在能力失效后保留草稿，只代表输入仍可继续编辑；是否允许保存仍以当前后端能力为准。版本比较继续使用原草稿对应的版本，未绕过并发保存保护。

## 开发自测

| 检查 | 结果与边界 |
| --- | --- |
| 渠道相关回归 | 52 通过、0 失败。复用已通过的记录，并核对本轮 17 个 Web 文件的摘要与发布内容一致；两项子集复跑不重复计入总数。 |
| 修改范围 lint | 0 错误、0 警告。 |
| 完整 Web 类型检查 | 本地通过；发布 CI 对固定 9f1bd154 再次通过。 |
| 桌面／窄屏交互 | 1440px、390px 的编辑、能力过期和恢复检查通过。失效期间提交写入数为零，草稿与选择保留。 |
| 390px 表单补充检查 | 浅色、深色两主题通过。七个关键字段可操作且值保留；保存按钮位于视口内、不被遮挡，可通过 Shift+Tab 获得焦点；无横向溢出和页面错误。未点击保存，配置写入数为零。 |
| 发布认证回归 | 11 通过、0 失败。 |
| 发布真实 Chromium 恢复检查 | 18 个场景通过，共 48 次受控请求。 |
| 静态构建与包体 | 构建、归档校验通过；首屏 gzip 499,783 bytes，预算 786,432 bytes。 |

七个窄屏字段为上游模型、优先级、权重、超时、最大输入 tokens、最大输出 tokens、合同版本。最终截图已实际检查，截图和浏览器原始收据保留在开发机的 scratch 证据目录。首次补充检查遇到本地夹具服务未启动，原失败记录保留；启动本轮夹具后完成检查并关闭临时服务。

主要证据入口：

- `scratch/channel-workspace-20261118/final-source-manifest.json`
- `scratch/channel-workspace-20261118/capability-management-final.json`
- `scratch/channel-workspace-20261118/capability-lint-final.json`
- `scratch/auth-recovery-return-20261118/typecheck-channels-final-v2.json`
- `scratch/channel-workspace-20261118/browser-final-v2/result.json`
- `scratch/channel-workspace-20261118/browser-fields-390/result.json`

## Key 创建、调用、结算和撤销

新增用例为 `dashboard_key_four_protocol_http_lifecycle_and_settlement`，位于 [native_model_http.rs](https://github.com/TokenNotIncluded/lmm-api-source/blob/97c50253122b8eb305518882d692fab9ae56c2e6/services/core/tests/native_model_http.rs)。

测试通过 reqwest 和真实 loopback TCP 访问正式的管理认证路由与模型路由，传入真实连接来源信息。Identity、Charging 和 ledger 使用新建的隔离 PostgreSQL。测试运行在测试二进制中，没有启动发布版 Core 主程序。

测试先通过管理 API 创建有效期十分钟、额度为 512、仅一次展示密钥的 Key，并限制可用模型、分组和来源地址。创建后的再次读取明文操作被拒绝。随后分别调用以下四个入口：

| 协议 | 入口 | 本轮结果 |
| --- | --- | --- |
| OpenAI Chat Completions | `POST /v1/chat/completions` | HTTP 200；响应内容与用量正确；独立 request ID；结算 49 测试 credits。 |
| OpenAI Responses | `POST /v1/responses` | HTTP 200；响应内容与用量正确；独立 request ID；结算 49 测试 credits。 |
| Anthropic Messages | `POST /v1/messages` | HTTP 200；响应内容与缓存用量正确；独立 request ID；结算 49 测试 credits。 |
| Gemini | `POST /v1beta/models/native-model:generateContent` | HTTP 200；响应内容与用量正确；独立 request ID；结算 49 测试 credits。 |

上述金额来自固定测试定价，不表示真实模型价格。每笔请求均核对数据库中的 settled 状态、Key 归属、输入／输出用量、已关闭预留及捕获账本记录。四笔合计 196，Key 剩余额度为 316；账户测试余额从 1,000 变为 804，预留余额归零。

通过管理 API 撤销 Key 后，用同样有效的协议请求再次访问四个入口。四次请求全部在认证阶段被拒绝；上游调用次数仍为每个供应商一次，计费请求仍为四笔，账本没有新增条目。全部 journal 的分录合计为零。

### 首轮失败及修正

首次执行为 1 通过、2 失败，退出码 101。Chat 和 Responses 已成功结算；新用例与既有正向用例在 Anthropic 合成响应解码处失败。当前解码合同要求 full response 包含 `role: "assistant"`，原测试夹具缺少该字段。

本轮只补齐该合成响应的 role／model，并让撤销后的 Anthropic 请求保留相同的版本头。生产解码和计费实现未修改。

第二次定向运行包含新增生命周期用例及两项既有 native_model_http 回归：**3 通过、0 失败，退出码 0**。完整运行耗时 10.903 秒，测试本体 5.98 秒；运行前后 1,109 个输入摘要一致，临时 PostgreSQL 正常停止，临时数据目录已清理。

实际命令：

```sh
cargo test --locked --offline --jobs 1 \
  --manifest-path services/core/Cargo.toml \
  --test native_model_http -- --test-threads=1 --nocapture
```

该命令需要预先提供隔离测试 PostgreSQL；执行记录使用本轮私有数据库启动脚本和已有 Cargo 缓存。

- 首轮记录：`scratch/key-protocol-flow-20261119/run-01.json` 与同名日志。
- 最终记录：`scratch/key-protocol-flow-20261119/run-02.json` 与同名日志。
- 最终测试文件 SHA256：`257d57301ea9f01a3661789f8549e36bd3f8a39e71ac2b0a97a7f12c7ba3cdee`。
- 最终运行日志 SHA256：`b7d06f629e695914b55ec804eb0d756ca158a58febf1216eaa43accbb7545b54`。

## Web 发布

[部署 run 44363827572](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44363827572) 已成功，固定源版本为 9f1bd154，完成时间为 2026-11-19 02:02:33（Asia/Taipei）。本轮只选择 Web 组件。

| 目标 | 执行结果 | 运行态核验 |
| --- | --- | --- |
| arch-shadow | ok 28；changed 7；failed 0；unreachable 0 | 健康检查通过，实际返回 HTML 与选定制品匹配。 |
| ubuntu-shadow | ok 28；changed 7；failed 0；unreachable 0 | 健康检查通过，实际返回 HTML 与选定制品匹配。 |

发布流水线还通过了临时 HTTP 环境中的部署／回滚演练。Core 和 Extensions 未在此轮重新部署。随后提交的 97c50253 只修改测试；文档提交也不改变当前已发布的 Web 身份。

脱敏发布回执：`scratch/channel-workspace-20261118/deploy-result.json`。

## 剩余边界

1. **登录态线上业务验收未完成。** 当前会话无法控制已登录的新架构浏览器，因此没有在生产管理页面创建真实 Key，也没有执行真实供应商调用及线上资金核对。受控 UI 检查、隔离数据库集成测试和 Web 健康回读分别证明各自范围，不能替代这条真实用户路径。
2. **全库 CI 尚未全绿。** [97c50253 的 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44368105622) 在 Web 全库格式检查处失败，涉及 62 个本轮未改动文件。与[原基线 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44348323101)逐项比较，原 63 项格式问题中，本轮解决了 management-review-fixtures.ts，剩余 62 项全部已在原基线存在，没有新增格式失败。原基线 Rust／Go 检查也有失败；本报告不据专项通过宣称全库检查通过。
3. **本轮生命周期用例覆盖非流式四协议请求。** 它验证真实管理认证、模型转发和账本链路，供应商由合成服务代替；不据此声称所有真实供应商、模型和流式行为均已完成业务验收。

后续继续处理可控登录态的线上 Key／供应商／结算路径和既有全库 CI 问题。当前状态与后续阻塞统一维护在 [progress.md](../progress.md)，机器可读摘要见[结构化证据](lmm-channel-key-protocol-2026-11-19.evidence.json)。

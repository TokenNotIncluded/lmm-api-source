# LMM API：渠道批量路由与模型估算器迭代

日期：2026-11-18（Asia/Taipei）。状态：源码已推送，开发自测通过；未部署，独立业务验收未执行。

本轮补齐渠道批量路由设置，并让请求费用估算器按照模型端点能力显示输入项。现有后端接口与计费公式保持原有语义。实现基于源码主线 `dc9bebfcb78dc6d2f956a663e84ae0b189b710a6`；并行任务的提交及原工作目录中未提交的工作均保留。

## 交付内容

### 渠道批量路由编辑

在“渠道”页选择多个渠道后，点击“编辑路由”，填写需要统一修改的字段并应用。修改作用于所选渠道的所有模型路由；留空字段不会发送，因此保留原值。

| 字段 | 接受的值 | 含义 |
| --- | --- | --- |
| priority | 0–65535 的整数 | 数值越小越优先 |
| weight | 0–4294967295 的整数 | 同一优先级中的请求分配权重 |
| timeout_ms | 1–3600000 的整数 | 请求超时时间，单位毫秒 |

优先级和权重允许填 0。小数、越界值和全部留空时不能提交。表单绑定所选渠道及配置版本：选择变化会清空草稿，版本变化会关闭失效表单；提交沿用 `expected_version`。版本冲突会刷新数据，修改不会自动重放。同一版本下的请求失败保留草稿，提交中的重复操作由现有 pending 保护拦截。

界面复用现有组件与设计风格。新增四条文案，通过现有翻译脚本同步至 en、fr、ja、ru、vi、zh-CN、zh-TW 七种语言。

代码：[be27b613](https://github.com/TokenNotIncluded/lmm-api-source/commit/be27b6133ac6d0ea5ae684631be6660da35dbcbc)。

### 按端点能力显示费用估算项

当模型声明了非空端点列表，且每个端点均属于 `systemone`、`embeddings`、`jina-rerank` 时，估算器只显示输入相关选项，隐藏输出、缓存及聊天/代码审查预设，并将相关输出与缓存数量按 0 计算。混合端点模型及未声明端点的模型保留完整选项。

这项改动减少不适用的输入，继续使用既有估算公式；它不改变 Core 的账单结算规则。

代码：[028eb4e3](https://github.com/TokenNotIncluded/lmm-api-source/commit/028eb4e3a885438fab0319cf2fa265be6998a58a)。

### 完整的本地预览夹具

实际截图检查发现，本地控制台预览遗漏了短信购买能力查询，导致页面显示错误横幅。夹具现为 `GET /api/hero-sms/capabilities` 返回 `purchases_available: false`。此修复仅补齐预览响应。

代码：[9aba1b87](https://github.com/TokenNotIncluded/lmm-api-source/commit/9aba1b872725c24d78a0c3d87d17f306d4f1a23d)。

## 验证结果

以下均为开发自测，计数包含已有回归用例；没有将重复运行累加为新的测试项。

| 检查 | 结果 |
| --- | --- |
| 渠道交互及原生 API 合同测试 | 60 pass，0 fail，exit 0 |
| 模型详情与费用估算器测试 | 11 pass，0 fail，exit 0 |
| 完整 Web 类型检查 | 功能代码检查 exit 0；最终夹具修复后复查 exit 0，66.026 秒 |
| 修改文件格式与 lint | 通过 |
| 生产构建及内置包体检查 | `mise exec -- just build-web`，exit 0 |
| 首屏 gzip | 499,589 B；门槛为 786,432 B |
| 文档检查 | 整库 docs-only 检查及本轮文档 JSON、空白、相对链接核对通过 |
| Chromium 渠道页面检查 | 6/6 通过；六张最终截图均已实际查看 |

生产构建对应 `be27b6133ac6d0ea5ae684631be6660da35dbcbc`。后续仅增加本地预览夹具与交付文档；并行进入主线的网络回归脚本不属于本轮功能验证范围。包体数值是本次构建的结果，不作为性能提升的比较结论。

浏览器场景如下：

| 视口宽度 | 主题 | 语言 |
| --- | --- | --- |
| 1440 px | 亮色 | zh-CN |
| 1440 px | 暗色 | zh-CN |
| 390 px | 亮色 | zh-CN |
| 390 px | 暗色 | zh-CN |
| 390 px | 亮色 | fr |
| 390 px | 暗色 | ru |

浏览器加载实际修改后的组件，使用现有本地合成管理数据。检查了零值、无效小数阻止提交、纠正输入后恢复提交、Tab 焦点顺序、取消关闭、横向溢出及错误横幅。各场景输入框与按钮高度均为 44 px；文档宽度没有超过视口宽度。外部网络和真实后端请求被阻断，检查期间没有后端写入。

### 保留的失败与复跑记录

- 首次六场景的交互断言通过，但人工查看截图发现预览夹具缺失导致的错误横幅，因此没有把首次截图记为完成。补齐夹具后，增加“可见错误通知为空”的断言，重新运行六场景并逐张核对。
- 夹具修复后的第一次完整类型复查在 180 秒时限终止，没有返回类型诊断。关闭本轮预览服务后，单独复跑在 66.026 秒以 exit 0 完成。超时记录保留，未记作通过。

## 复核命令

在 `apps/web` 下执行：

```sh
mise exec -- bun test --preload ./scripts/test-preload.mjs --timeout 15000 src/features/channels/components/native-routing-management.test.tsx src/features/channels/native-api.test.ts
mise exec -- bun test --preload ./scripts/test-preload.mjs --timeout 15000 src/features/pricing/components/model-details.test.tsx
```

在源码仓库根目录执行：

```sh
mise exec -- bun run typecheck:web
mise exec -- just build-web
python3 -B scripts/check-docs-brand.py --docs-only
```

## 证据与交付位置

- [结构化证据](lmm-channel-routing-2026-11-18.evidence.json)：命令、退出状态、构建数值、浏览器场景、截图 SHA256、源码 Git 对象身份、历史超时及适用范围。
- [Google 云盘交付目录](https://drive.google.com/drive/folders/1Ixnf5t8di1HmVyWJUu6F9L2bQ-jP2yL3)：本报告及结构化证据的副本。
- 开发机原始记录位于配方仓库的 `out/lmm-channel-routing-20261118/`，包括日志、浏览器脚本、首次截图和最终截图。估算器早期测试输出保留在其 ssmcp 任务记录中，未另存为磁盘日志。

本轮完成了上述前端改动的开发与自测。真实 Core/PostgreSQL 联调、外部模型调用、真实资金核账及独立业务验收未执行；生产环境未部署本轮代码。

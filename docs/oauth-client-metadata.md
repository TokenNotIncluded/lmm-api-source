# OAuth 模型客户端的动态元数据

这里描述 `packages/` 中的模型调用客户端及 `apps/cli` 的目录读取，不是 Go 扩展宿主的模块管理。服务端提供 `GET /api/oauth2/catalog`，插件以各自 OAuth 授权读取，保留现有分组和调用权限校验。

`schema_version=1` 的模型新增可选 `metadata`：`context_window`、`max_input_tokens`、`max_output_tokens`、`input_modalities`、`output_modalities`、`reasoning`、`tool_call`、`source`。未知保持 null。存在服务器元数据时，不通过打包的模型 ID 白名单接纳新模型，也不使用旧快照填补服务器未知值。完全省略 metadata 的旧服务器保留原兼容路径。

`upstream_model` 是原始请求 ID；规范 `id` 保持分组绑定。`pricing` 保留币种、计量单位、输入/输出/缓存/按次费率和 group/trust 倍率；有效价格已经包含倍率，不能再次相乘。`native_cost` 仅用于完整的 USD/百万 token 四方向费率。缺失价格不是零价，按次或表达式费用也不能假装成 token 单价。

宿主只提供一个上下文预算时，使用总上下文与输入上限中的较小值；只知输入上限时将它用作保守预算，不与输出相加。动态适配目前使用受支持的文本传输；reasoning 标志本身不能证明供应商特定思考参数或图像传输兼容。

| 客户端 | 同步与呈现 |
| --- | --- |
| Pi | 原生模型刷新、`/lmm-prices`；新模型需要已知文本能力、工具支持、限制和完整 native_cost，否则价格报告只读展示 |
| DSH | 通过固定 `vendor/pi-lmm-provider` 共享 Pi 逻辑；协调刷新 TTL 60 秒，显式刷新失效旧快照 |
| OpenCode 1/2、Magpie | 共用 opencode-lmm-auth 的目录接纳；发现/配置或 reload 拉服务器，显示原始 ID、限制、价格和倍率；缺失必要数据由诊断说明 |
| VS Code | 发现时同步，生成前重验；目录选择器显示全部授权条目，Chat 只使用已知限制或显式覆盖的文本模型；tooltip 保留服务器价格和未知值 |
| Zed | settings/configure/start 拉目录；运行中桥接在发现和调用时刷新并重写受管 LMM 模型块；其他配置不变，显式覆盖使用 `--models`；原生选择器无价格字段，完整价格在 catalog/桥接发现中 |
| Codewhale | 原有实现已逐请求刷新并原样透传完整目录；本次增加变更、删除和未知价格回归，不改变传输 |
| CLI | `lmm models` 保留并输出 metadata、缓存价格及 group/trust 倍率；CLI 不是 Magpie 插件宿主 |
| lmm-scripts | 安装/启动入口不维护模型目录，继续委托相应客户端，不需新增模型 ID |

成功的空目录删除模型；网络失败不能伪装为空目录成功。目录同步不触发付费生成。Zed 的宿主可能需要重新打开模型选择器才能展示设置变化。插件升级一次后可随服务器目录演进；新协议或宿主接口变化仍可能需要插件更新。

开发者检查位于各子仓库：Pi/OpenCode 的 catalog 测试，DSH provider/package 测试，VS Code metadata/provider 测试，Zed server-metadata/bridge 测试，Codewhale provider 测试，以及 CLI OAuth 序列化测试。子仓库必须先推送并核对可达提交，再更新父 gitlink。源码自测、真实宿主验收、包发布与生产部署分别报告。

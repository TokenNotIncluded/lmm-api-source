# OAuth 模型客户端的动态元数据

这里描述 `packages/` 中的模型调用客户端及 `apps/cli` 的目录读取，不是 Go 扩展宿主的模块管理。服务端提供 `GET /api/oauth2/catalog`，插件以各自 OAuth 授权读取，保留现有分组和调用权限校验。

## 原生客户端与有限额度

Core 为下列公共原生客户端分别登记 OAuth Client ID。它们均使用授权码流程、
PKCE S256 和 `http://127.0.0.1:<动态端口>/oauth/lmm/callback`，
`resource` 固定为 issuer 加 `/api/oauth2`。

| 客户端 | Client ID | 有限额度配置键 |
| --- | --- | --- |
| DSH | `lmm-dsh` | `provider_max_credits."lmm-dsh"` |
| OpenCode | `lmm-opencode` | `provider_max_credits."lmm-opencode"` |
| CodeWhale | `lmm-codewhale` | `provider_max_credits."lmm-codewhale"` |
| VS Code | `lmm-vscode` | `provider_max_credits."lmm-vscode"` |
| Zed | `lmm-zed` | `provider_max_credits."lmm-zed"` |

五个客户端默认关闭；仅显式配置的客户端可申请模型授权。每个值是该客户端一次授权族的
整数 credits 预算上限，范围为 1–9007199254740991。未知 Client ID、零值、负值或超界
额度会拒绝配置。Pi 继续使用独立的 `pi_max_credits`，其 Client ID 为 `lmm-pi`。

在已有 Core TOML 的 `[identity.oauth2]` 中配置上限，同时配置实际模型服务。
下面为配置片段，issuer 应与已有 `identity.web.origin` 完全一致；示例额度按安装策略调整：

```toml
[identity.oauth2]
issuer = "https://api.example.invalid"
groups = ["default"]
access_ttl_seconds = 900
family_ttl_seconds = 2592000
provider_max_credits = { "lmm-dsh" = 10000, "lmm-opencode" = 10000, "lmm-codewhale" = 10000, "lmm-vscode" = 10000, "lmm-zed" = 10000 }

[model]
config_file = "/etc/lmm.conf.d/core-model.toml"
```

`provider_max_credits` 也可写为 `[identity.oauth2.provider_max_credits]` 子表；
未列出的客户端保持关闭。停用客户端后拒绝新授权和刷新，仍允许撤销已有令牌。
原生客户端无需 `managed_refresh_keyring_file`，
授权和刷新请求也无需 `installation_id`、`display_label` 或 `rotation_id`。
CC Switch 与 Magpie 的安装绑定及刷新协议继续按各自 managed 客户端合同配置。

客户端首次申请 `catalog:read balance:read usage:read models:invoke`。用户在浏览器
完成登录及账号/group 确认，并按账户策略验证 MFA 后，显式同意一个不超过该客户端
上限的有限预算。
同一授权族在刷新前后累计消费与预留，继续接受个人账户及所选 group 的权限检查。
已知的可选 MCP scope 可出现在初始请求中，Core 签发范围遵循
[客户端 OAuth2 授权](core-identity.md#客户端-oauth2-授权)。

### 组件 77 升级到 78

`provider-clients` 是第 78 个 Core schema 组件，扩展 OAuth 客户端及计费来源约束。
已有精确组件 77 库通过固定 `core-provider-clients-v1` procedure 升级；
实现与前置指纹见 [owner 升级实现](../services/core/src/core_schema/upgrade/provider_clients.rs)。
普通启动只校验安装契约。

在已准备匹配二进制、备份恢复材料和完整升级请求的离线维护窗口，使用独立
installer/owner 配置执行既有入口：

```sh
lmm-core --config /absolute/private/core-owner.toml upgrade-db < /absolute/private/core77-to78.json
```

请求中的 `procedure` 为 `core-provider-clients-v1`，其余数据库身份、角色和前后置
摘要字段沿用 [UpgradeRequest](../services/core/src/core_schema/upgrade.rs)。
保留原请求、退出状态与 `core.upgrade-db.v1` 回执，并核对编译时固定的目标契约。
runtime 继续使用最小权限角色。实际开发验证、独立验收和部署状态在
[架构进度](progress.md)记录。

## 模型目录元数据

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

---
name: audit-hardcoded-values
description: 审查 LMM API 中写死的地址、超时、凭据、协议字段和界面文字。用于判断一个值由谁决定，追踪现有配置是否传到实际调用方，并以最小改动修复有部署或用户影响的问题。
---

# 审查和消除硬编码

先问一个能验证的问题：换部署、换渠道或换界面语言时，谁有权改变这个值？再从实际调用点逆向追到它的来源。只有当有权修改的人无法通过现有入口改变行为，或敏感值进入了错误的载体时，才记录缺陷。

在父仓从 `generated/lmm-api-source/` 进入实现，读取适用的 `AGENTS.md`。以下示例以公开源码的固定版本说明输入到调用方的链路。复查时记录当前目标提交；本地 HEAD、远端跟踪引用和远端分支可能各自指向不同版本。

## 先判断值的归属

| 类别 | 谁决定它 | 项目实例 | 审查与处理 |
| --- | --- | --- | --- |
| 协议常量 | 与调用方共同遵守的接口契约 | Core 把 `POST /v1/chat/completions` 映射到 `chat`，也按 `ClientProtocol::Chat` 生成对应路径 | 保留路由及大小写，复用已有协议类型。把上游实例地址与客户端协议路径分别追踪；重复出现的路径可能同时承担路由注册与兼容性测试。 |
| 默认值 | 项目为省略该字段的用户选定 | 已配置数据库、未填 `database.acquire_timeout_seconds` 时取 3 秒，接受 1 至 30 秒 | 保留旧配置的缺省行为。先找现有覆盖入口，分别验证省略、合法显式值和越界值。显式零是否有效由字段契约决定。 |
| 运维配置 | 部署此实例的运维者 | `server.listen` 控制监听地址，示例与默认值为 `127.0.0.1:8080` | 复用 Core TOML 和已有验证；沿启动链检查显式地址是否传入监听器。接管已有套接字时，另外检查现有的地址匹配逻辑。 |
| 密钥或凭据 | 对应服务的凭据持有人 | `database.url_file` 保存私有文件的引用，文件中的连接串可能带密码 | 示例保留路径，测试使用隔离临时凭据。审查配置、日志、错误、测试快照及报告中的敏感值流向，记录位置和规则名即可。 |
| 可翻译文字 | 用户选择语言，由翻译资源提供文本 | 渠道高级编辑器的 `t('Save changes')` 在简体中文资源中对应“保存更改” | 保留英文源键和既有字典，对屏幕按钮、提示、无障碍标签追踪渲染入口。HTTP 路径、配置字段和机器读取的错误码保留原值。 |

协议与 Web 实例可分别定位到[路由注册](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/model_http.rs#L559-L566)、[协议路径选择](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/model_http.rs#L1209-L1220)、[按钮调用](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/apps/web/src/features/channels/components/dialogs/advanced-custom-editor-dialog.tsx#L545)、[简体中文资源](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/apps/web/src/i18n/locales/zh-CN.json#L10213)和[语言资源加载器](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/apps/web/src/i18n/config.ts#L36-L52)。

## 沿现有入口追到使用点

1. **锁定实际影响。** 从用户操作、对外接口或启动入口开始。写出一个可观察差异，例如“设置连接池取连接等待时间为 7 秒后，池构造器应收到 7 秒”。搜索字段名、内部键以及消费函数；同一个数字在不同功能中可以有不同含义。
2. **核对输入和生命周期。** 查示例、类型、缺省、范围、加载函数及其调用者。Core 的 `main` 把选定路径交给 `config::initialize`，后者按进程一次读取 TOML，并拒绝混用进程环境中的 `LMM_CORE_*`。其解析结果使用这些名称作为内部键。指导运维时给出受支持的 TOML 字段，标明配置在哪个生命周期读取。[初始化入口](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/main.rs#L380) · [加载和读取](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L35-L96)
3. **比对每一次转换。** 对 `database.acquire_timeout_seconds`，TOML 解析补全默认值并验证范围，`database_acquire_timeout_seconds()` 读取对应键，`bounded_database_pool` 把秒转换为 `Duration` 并交给 SQLx 的 `.acquire_timeout(...)`。相邻的连接空闲时间和 SQL 语句执行时限各有用途；按消费 API 辨认需求能避免同时改变三个时间限制。[样例配置](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/config/core.example.toml#L12-L19) · [解析](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L578-L595) · [访问器](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L3978-L3980) · [连接池](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L1457-L1478)
4. **在真实使用处分辨配置和敏感内容。** `server.listen` 经 `listen_address()` 交给启动函数；无继承套接字时调用 `TcpListener::bind(address)`。同一启动函数读取 `database_url()`，使用解析出的连接串建池；正常 TOML 的 `url_file` 经过私有文件读取，空内容会报错。寻找“写死”时，公开的路径名和路径指向的凭据分别处理。[启动入口](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/main.rs#L399-L426) · [监听分支](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/main.rs#L838-L841) · [私有文件读取](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L3995-L4018)
5. **修复最近的错误来源。** 配置已经存在但调用方仍用字面量，就接回现有参数；条件允许时删去错误旁路。已有配置归属 Core 时沿用 Core 权限和写入方式。遗漏翻译时，使用既有 `react-i18next` 入口与翻译补丁流程；动态键按现有静态键清单登记，同步七种语言并核对变量。把配置项加入表单的前提是产品确实需要授予该操作者写权限。[Web 文案约定](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/AGENTS.md) · [翻译维护流程](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/.agents/skills/i18n-translate/SKILL.md)

## 有针对性地找候选和验收

从源仓运行以下只读查询。第一条能将示例、解析与消费位置放在一起；第二条只列 Web 调用点文件，便于区分按钮文字与协议标识。

```sh
git grep -n -E 'acquire_timeout_seconds|LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS|\.acquire_timeout\(' -- config/core.example.toml services/core/src
git grep -l -F "t('Save changes')" -- apps/web/src/features
```

凭据疑点使用仓库现有的 Gitleaks 配置。对已提交的变更，用实际提交替换下例的 `BASE` 和 `HEAD`，只扫描该段提交差异，并对输出脱敏。未提交的文件用 `gitleaks dir --redact=100` 指向受影响的源码目录。命中后判断值的来源和可用性；真实凭据需由持有人撤销或轮换，移动到新文件无法撤销已经泄漏的旧值。[仓库规则](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/.gitleaks.toml) · [Gitleaks 官方用法](https://github.com/gitleaks/gitleaks#usage)

```sh
gitleaks git --redact=100 --log-opts="BASE..HEAD" .
```

用最靠近行为的观察点收尾。配置改动至少覆盖旧配置省略字段、一个非默认合法值及契约要求拒绝的值；在消费者断言实际解析出的地址、持续时间或构造器参数。现有 `database_acquire_timeout_defaults_and_bounds_are_explicit` 覆盖解析和边界，不能单凭该测试报告连接池运行时已经验收。协议改动检查原有方法、路径和序列化；凭据改动使用临时私有文件，验证读取失败和日志脱敏；界面文字改动切换相关语言，检查完整句子、回退与命名占位符。[现有配置测试](https://github.com/TokenNotIncluded/lmm-api-source/blob/f5ba3a2956162c93a8d9d05ed650a3514799cee1/services/core/src/config.rs#L5031-L5059) · [i18next 插值规则](https://www.i18next.com/translation-function/interpolation)

交付每项实际发现时，写清 **位置与被影响的调用方、值的归属、触发问题的输入、最小修复、验证证据**。对有意保留的协议或默认值给出一处契约或测试依据。分别标明“已阅读源码”“已运行测试”“已观察到运行行为”，让下一位维护者知道哪些结论可以复用。

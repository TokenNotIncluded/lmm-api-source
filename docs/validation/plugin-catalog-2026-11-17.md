# LMM 插件模型目录修复与发布验收

日期：2026-11-17（Asia/Taipei）。范围：`lmm-api-source/packages` 下六个提供方插件和 `lmm-scripts` 安装器。开发、构建、真实宿主验收与打包均在用户开发机完成。

## 交付结果

七个子仓库均已更新并推送；以下 GitHub Release 已发布，资产摘要与验收包一致。Pi 同时完成 npm Trusted Publishing，`alpha` 指向 `0.1.0-alpha.6`，实际从 registry 下载的 tarball 与验收包逐字节一致。

| 组件 | 最终版本 | 发布代码提交 | 发布入口 |
| --- | --- | --- | --- |
| codewhale-lmm-provider | 0.1.0-alpha.3 | [2b0d6b22](https://github.com/TokenNotIncluded/codewhale-lmm-provider/commit/2b0d6b22af069ac8685dd677d99dbbfd2340514e) | [GitHub Release](https://github.com/TokenNotIncluded/codewhale-lmm-provider/releases/tag/v0.1.0-alpha.3) |
| dsh-lmm-provider | 0.1.0-alpha.8 | [2f11acf7](https://github.com/TokenNotIncluded/dsh-lmm-provider/commit/2f11acf7419513dd549e4afe8515f44a95fb1123) | [GitHub Release](https://github.com/TokenNotIncluded/dsh-lmm-provider/releases/tag/v0.1.0-alpha.8) |
| lmm-scripts | 2026.11.17 | [a19299e8](https://github.com/TokenNotIncluded/lmm-scripts/commit/a19299e8cf8c3ab87361d792731156950452dc8b) | [GitHub Release](https://github.com/TokenNotIncluded/lmm-scripts/releases/tag/v2026.11.17) |
| opencode-lmm-auth | 0.1.4 | [26b3a60d](https://github.com/TokenNotIncluded/opencode-lmm-auth/commit/26b3a60df95d13580e03a56e8d784ae06aa89cdb) | [GitHub Release](https://github.com/TokenNotIncluded/opencode-lmm-auth/releases/tag/v0.1.4) |
| pi-lmm-provider | 0.1.0-alpha.6 | [358960d0](https://github.com/TokenNotIncluded/pi-lmm-provider/commit/358960d02828eb385aa35514c5c56470a4349572) | [GitHub Release](https://github.com/TokenNotIncluded/pi-lmm-provider/releases/tag/v0.1.0-alpha.6) |
| vscode-lmm-provider | 0.1.5 | [23a278af](https://github.com/TokenNotIncluded/vscode-lmm-provider/commit/23a278af3743afdd09dad7f98daa666b5188efd8) | [GitHub Release](https://github.com/TokenNotIncluded/vscode-lmm-provider/releases/tag/v0.1.5) |
| zed-lmm-provider | 0.1.3 | [f1826810](https://github.com/TokenNotIncluded/zed-lmm-provider/commit/f18268100b0b11b9d30d2dde010948db7c2934f4) | [GitHub Release](https://github.com/TokenNotIncluded/zed-lmm-provider/releases/tag/v0.1.3) |

Pi 的发布标签固定为 `358960d02828eb385aa35514c5c56470a4349572`；随后只修复了未打包的发布校验器，因此源码父仓库的 Pi gitlink 指向 `737f59b665fd53d7d94c32f3a6fbc18b2da91c00`。DSH 内嵌 Pi 继续固定到已验收的 `358960d` 运行时。没有移动既有标签或替换已发布资产。

Zed `0.1.3` 与 VS Code `0.1.5` 修正了打包文档中的旧安装指令；其编译后的运行时代码分别与已验收的 `0.1.2`、`0.1.4` 逐字节相同。

## 模型列表如何生成

可用模型来自账户授权目录。这个目录回答“账户能通过哪个分组调用哪个上游模型”。models.dev 是参考元数据，回答“这个模型通常有哪些能力、容量和参考价格”。插件将两者合并，保留账户原有的模型 ID、分组和请求路由。

因此，公共元数据匹配失败、名称格式变化或出现新模型，不再成为删除账户模型的理由。模糊匹配只用于补齐元数据，不重写真实请求中的模型名，也不扩大账户权限。

| 原问题 | 本次行为 |
| --- | --- |
| 静态模型白名单、能力快照与固定供应商列表限制模型发现 | 删除模型白名单与旧静态快照；Pi 动态读取宿主 SDK 提供方，再由公共缓存补齐 |
| 名称必须精确一致才能获取能力 | 支持提供商前缀、大小写、分隔符、日期别名、词序和有限拼写差异；保留数字版本/模型尺寸区别，歧义匹配返回未知 |
| 元数据未知时直接丢弃模型 | 继续保留已授权且协议可用的模型，明确标注未知元数据与估算容量 |
| 大目录超过约 2 MiB 后整组失败 | 账户模型目录响应上限提高到 32 MiB；OAuth 等小响应保留原限制 |
| models.dev 中 0 容量导致整组失效 | 非正容量视为未知，各宿主使用明确的正数预算回退，不使其它模型一起消失 |
| 推理、图片输入、缓存价格在 Magpie 映射时丢失 | `provider.models` 返回原生 SDK 模型结构，保留嵌套能力、成本和 API URL；原生入口保留自身 provider ID |
| 参考价格覆盖真实账户价格 | 账户显式价格优先；未知估价有可见说明，参考价格不参与服务端实际计费 |
| 向量、重排、音频等专用模型被当作普通聊天模型 | 按明确类型、输入/输出模态及宿主协议能力区分；不将宿主不支持的条目伪装成可聊天模型 |
| DSH 内嵌代码和安装器仍引用旧版本 | 更新 DSH 的真实 Pi 子模块指针；Codewhale 安装器下载完整新包，并更新 helper 的不可变提交和校验值 |
| 发布和说明仍依赖已删除文件 | 修复 Pi 发布校验器必需文件、VS Code 旧 VSIX/快照脚本引用、Zed 旧 npm 安装路径 |

共享运行时由 `scripts/shared/models-dev.ts` 维护，`scripts/sync-provider-metadata.mjs --check` 已确认 5 份独立插件副本无漂移。DSH 通过已固定的 Pi 子模块复用运行时；安装器不携带没有被使用的另一份元数据实现。

## models.dev 自动缓存与优先级

- 数据源：[https://models.dev/api.json?type=all](https://models.dev/api.json?type=all)。
- 默认缓存有效期 24 小时，支持 ETag/304、原子写入、过期缓存回退和进程内并发下载合并；刷新失败有短暂冷却，避免每次列模型都重复请求。
- 可用 `LMM_MODELS_DEV_CACHE` 指定缓存路径，`LMM_MODELS_DEV_TTL_MS` 调整缓存有效期。
- 账户服务器给出的明确能力、容量和价格优先；用户/宿主已提供的合法配置与公共元数据只用于补齐。
- 未知容量仍可列出模型，但估算值不等于上游承诺。以 OpenCode/Magpie 为例，完全未知时采用 32,768 上下文、4,096 输出的保守估算，并在名称/选项中标明。

实际冷缓存运行于 **2026-11-17 18:14:50.398（Asia/Taipei）** 下载官方元数据并写入完整缓存和 ETag；不是只依赖预置测试 JSON。此份数据包含 **326 个提供商、10,010 条提供商模型记录、4,404 个去重模型 ID**，原始数据 SHA-256 为 `3a2cb0e0a51da323b166d2effd27e86a22c609936b2982bd3342873a083d3073`。

## 真实 Magpie 安装验收

使用开发机实际 **Magpie 0.1.2512** 和独立配置目录安装最终发布包，完成本地可信 HTTPS OAuth/PKCE、插件加载和模型列表读取。目录来自上述完整公共语料，另加入 3 个模糊别名、2 个未知未来模型和第二分组的 10 条重复路由，共 **4,419 条账户形状路由**。

| 真实安装入口 | 目录路由 | 预期支持聊天路由 | 实际列出 | 缺失支持路由 | 意外多出 | 明确不支持 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| OpenCode 原生 `/magpie` | 4,419 | 4,002 | 4,002 | 0 | 0 | 417 |
| OpenCode 默认入口 | 4,419 | 4,002 | 4,002 | 0 | 0 | 417 |
| Pi 插件 | 4,419 | 4,002 | 4,002 | 0 | 0 | 417 |

核对的是排序后的模型 ID 集合完全相等，同时检查分组数量与集合哈希；不只比较总数。417 条是明确的专用类型、非文本模态或不支持协议，不是因元数据名称未命中而遗漏。

两个 OpenCode 入口对 2,933 个明确匹配的公共元数据逐项核对 reasoning/image，结果 0 不一致；所有 4,002 个模型均保留账户输入、输出、缓存读取、缓存写入价格与正确 HTTPS API URL。三个 Qwen/Kimi/Step 模糊别名匹配正确，两个未知模型继续可见。Pi 同样通过能力、价格和别名检查；其宿主列表没有公开 URL 与逐模型工具能力字段，未将这两项写成已验证。

最初的缺陷有真实失败证据：旧插件会漏掉未知新模型；完整目录曾触发旧响应大小限制；放宽目录后又发现零容量记录会使整组失败；最终宿主核对还发现了扁平化字段造成的元数据丢失。这些具体问题均已修复并在最终制品复验。

**验收范围：**上述 4,002 是完整公共语料构造的支持路由数量，不是用户生产 LMM 账号当前模型数。没有可用的生产 LMM 授权，因此未进行真实账户目录读回；未改动用户原有 Magpie 配置，付费推理请求为 0。DSH 的流式检查只调用本地合成响应。

## 其它插件与定向检查

| 范围 | 已完成检查 |
| --- | --- |
| 共享元数据模块 | 11 项缓存/匹配检查、严格类型检查、5 份副本一致性检查 |
| OpenCode / Magpie | catalog、授权与账户隔离既有检查；最终原生/默认宿主契约 14/14；typecheck、build、真实最终包验收 |
| Pi | 初次全量 211/212；唯一失败为要求删除未知模型的旧断言，修正后相关能力/目录 19/19 通过；类型与文档检查；发布校验器回归 3/3 |
| Codewhale | 初始整合 44 项，后续相关 19 项，5 MiB 目录回归 |
| Zed | 初始整合 26 项，后续相关 4 项，5 MiB 目录回归，类型与打包 |
| VS Code | 初始整合 39 项，后续相关 21 项，5 MiB 目录回归，编译与 VSIX 打包；旧静态 JSON 不在包中 |
| DSH | 17 项、typecheck、build；实际 DSH Web `0.2.2-rc.1` 安装最终 tarball，完成启动、OAuth、目录、合成流式和登出 |
| 安装器 | Codewhale 32 项与 Bash 语法检查；完整 provider archive 和不可变 helper 校验值 |

Codewhale、Zed、VS Code 还各自用同一份 4,419 路由语料跑过适配器，均保留 4,419 条账户记录，识别 4,002 条聊天路由和 417 条明确不支持路由；路由 ID、上游 ID 和账户价格不变，聊天预算均为正。该项是适配器语料验证，不冒充这三个宿主的交互式全流程验收。各阶段检查按实际执行范围记录，不把不同阶段的通过数相加伪装成一次全量测试。

## 发布渠道与安装

- Pi：[npm 0.1.0-alpha.6](https://www.npmjs.com/package/@tokennotincluded/pi-lmm-provider/v/0.1.0-alpha.6) 已发布。使用 `@alpha` 或明确版本；`latest` 保持既有 `0.1.0-alpha.2`。可信发布及 registry 完整性检查：[Actions 44064957826](https://github.com/TokenNotIncluded/pi-lmm-provider/actions/runs/44064957826)。
- OpenCode/Magpie：使用 Release 的提交寻址 `.tgz`；`release.json` 和 `SHA256SUMS` 保持原安装器合同。解压后，实际测试过的原生入口为 `magpie plugin add <解压目录>/package/dist/magpie.js`，登录为 `magpie plugin login lmm-magpie`，读回为 `magpie plugin list --json`。统计对应提供商 `providers[].models.length`。
- DSH：GitHub Latest 指向 alpha.8，并提供安装器需要的 `dsh-lmm-provider.tgz` 别名，与规范包字节完全相同。
- Zed：README 已改为安装新 GitHub tarball；npm 当前仍为旧 `0.1.1`，不要用未指定新制品的 npm latest 代替本次发布。
- VS Code：安装 Release 的 `lmm-copilot-provider-0.1.5.vsix`。正常 `vsce ls-publishers` 检查返回空，没有可用的已配置发布者；本次未发布到 Visual Studio Marketplace。
- DSH 与 Zed 未配置可用的 npm 可信发布流程，本地 npm 身份检查为 E401；未改动凭证或权限。GitHub 安装包均已发布并读回校验。

## 制品 SHA-256

| 制品 | SHA-256 |
| --- | --- |
| tokennotincluded-codewhale-lmm-provider-0.1.0-alpha.3.tgz | `54b3ecaa85ce1a6de3573a4e14b030eda45338c2312fc7a195b8bdaefcac8da4` |
| tokennotincluded-dsh-lmm-provider-0.1.0-alpha.8.tgz | `7cf932b0a79bd61c1b3c10b044d5464f1c85acbd781641aaf03617953826e8bc` |
| lmm-scripts-2026.11.17.tar.gz | `528647adbacb5d748b33a76793f503116478fcb10a0b65373f67100afb4c20f1` |
| opencode-lmm-auth-26b3a60df95d13580e03a56e8d784ae06aa89cdb.tgz | `58f7527543c63d9a69c9622282bd9191d8039da9be99ca3ac6f925d39e403f87` |
| tokennotincluded-pi-lmm-provider-0.1.0-alpha.6.tgz | `853f1cc821184793cd2a29129b6266a55422bc61e7169c03af3a8b3206161d58` |
| lmm-copilot-provider-0.1.5.vsix | `81a9adf0a761f00a894c8164e006adce6fef205084018616c20b116b3bf16cd8` |
| tokennotincluded-zed-lmm-provider-0.1.3.tgz | `2a8e4bfd930bb304d4f77d9c707a51f35b994f0546e874dd88acaa52a619573f` |

## 父仓库 gitlink 与证据

本报告所在源码提交更新以下七个子仓库指针；外层 `lmm-api` 的 `generated/lmm-api-source` 指针随后推进至该源码提交。现有其它开发任务改动保持在原位置，没有混入本次提交。

| gitlink | 固定提交 |
| --- | --- |
| packages/codewhale-lmm-provider | `2b0d6b22af069ac8685dd677d99dbbfd2340514e` |
| packages/dsh-lmm-provider | `2f11acf7419513dd549e4afe8515f44a95fb1123` |
| packages/lmm-scripts | `a19299e8cf8c3ab87361d792731156950452dc8b` |
| packages/opencode-lmm-auth | `26b3a60df95d13580e03a56e8d784ae06aa89cdb` |
| packages/pi-lmm-provider | `737f59b665fd53d7d94c32f3a6fbc18b2da91c00` |
| packages/vscode-lmm-provider | `23a278af3743afdd09dad7f98daa666b5188efd8` |
| packages/zed-lmm-provider | `f18268100b0b11b9d30d2dde010948db7c2934f4` |

可复核的脱敏收据与本报告一同保存：

- [真实 Magpie 安装、目录集合和元数据](plugin-catalog-magpie-2026-11-17.json)
- [其它适配器、DSH 宿主、发布资产与安装器](plugin-catalog-providers-2026-11-17.json)
- [Pi npm registry 独立读回](plugin-catalog-npm-2026-11-17.json)

收据保留开发机完整证据路径与摘要；不包含生产凭证、原始授权日志或私钥。GitHub 发布、npm 发布、本地宿主验收分别记录；本轮没有部署生产服务端。

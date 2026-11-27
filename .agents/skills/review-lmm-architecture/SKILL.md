---
name: review-lmm-architecture
description: 在 LMM API 审查或改进 Core、Extensions 与 Web 的职责归属、跨模块依赖、循环依赖、重复实现或难以维护的调用链时使用。适用于架构评审和有明确范围的重构。
---

# LMM API 架构改进

从一个具体操作开始。写出“谁发起，谁校验，谁写入，失败后向谁查结果”，然后沿真实入口找代码。架构问题要能说明一个可观察的后果，例如新增插件必须同步修改两张名单，漏改其中一处会使已经启用的插件缺少入口。

## 定位代码和数据的归属

从父仓进入 `generated/lmm-api-source/`。读源码仓 `AGENTS.md` 和 `docs/core-migration.md`。记录 `git rev-parse HEAD`、相关文件的本地差异以及所评审的提交。如果评审对象为远端 main，用 `git ls-remote origin refs/heads/main` 取得 SHA，用 `git show <SHA>:<路径>` 读取该版本。共享工作目录有并行修改时，独立注明本地测试所用版本。

| 要回答的问题 | 到哪里找证据 | 改动要保留的行为 |
| --- | --- | --- |
| 谁决定身份、付款方、可用预算及最终账本？ | `services/core/`、`docs/core-protocol.md` 和 `contracts/proto/lmm/core/v1/` | Go 经私有 socket 调用授权范围内的 RPC。模型调用、预留、结算和重试仍引用 Core 的权威状态。 |
| 谁记录供应商回调、注册插件和装配存储？ | `services/extensions/internal/modules/` 和 `services/extensions/internal/composition/`，配合 `docs/plugin-development.md` | 扩展保存自己的业务记录。宿主装配所需接口和凭据，插件通过业务 RPC 请求 Core 处理资金。 |
| 谁决定菜单可见，谁决定操作能否执行？ | 从 `apps/web/` 下的页面、请求函数追到服务端处理入口，再对照插件目录返回的数据 | 页面反映能力及启用状态。服务端逐次校验权限。直接访问页面或接口也要符合约定。 |

依赖关系会出现在导入、运行时注册、配置、RPC 和数据库写入中。只画本次行为涉及的边，每条边注明起点、终点、接口及持有的数据。TypeScript 的导入图查不到 Go 描述符里的依赖，Go 包列表也看不到 gRPC 方法后面的事务。

## 选择一处值得改的结构

1. **找重复决定。** 用 `rg -n -F '具体字段或符号' apps/web services contracts` 查生产调用方，再查测试、配置和旧数据读入路径。记录两处各自处理的输入、缺省值、授权、错误、重试和副作用。如果同一业务决定必须在两处同步修正，把决定留给已有所有者，让调用方消费其结果。
2. **验证重复是否有意。** 客户端输入提示和服务端鉴权有不同职责。协议生成文件应追到 proto 源，历史导入器应核对支持的旧格式。仓库的 `.jscpd.json` 将最低行数设为 50、最低 token 数设为 300，并排除若干文件模式。低于阈值的两份价格判定仍可能分歧。扫描命中时先比对行为，再决定合并、保留或删除。
3. **判断移除能否缩短一次真实修改。** 列出迁移前后的调用方和写入位置。优先去掉重复名单、转手参数的包装层或反向依赖。需要新接口时，用一个现存调用方和一次真实失败说明该接口保护了什么。拆开后若仍需同步发布两端才能工作，复查协议兼容性和拆分收益。
4. **按授权实施。** 评审任务交付证据、建议和复验方法。重构任务先迁移现有使用者，再删除失去用途的实现。复查按字符串注册的模块、旧配置和持久数据。受影响行为都有归属后停止扩大范围。

## 用插件注册验证一个判断

假设评审发现前端又维护了“support 依赖 store”的名单，先核对 `services/extensions/internal/composition/registry.go`。在核验的 main 上，`moduleRegistrations` 已声明这个 `Requires`，`SelectModules` 会验证目录。继续追踪管理 API 返回的 `requires` 及前端消费处。只有确认额外名单重复决定了同一依赖，才迁移该调用方；纯展示用的翻译标签可独立保留。

在源码仓根目录执行插件目录的针对性验证。

```sh
(cd services/extensions && mise exec -- go test -v -count=1 -run '^(TestRegisteredModuleSelection|TestSelectionRejectsCycleEvenWhenModulesAreUnselected|TestModuleRegistryBindsSelectedRoutes)$' ./internal/composition)
```

核对输出中每个具名用例都实际执行。只选 support 被拒，显式选 store 与 support 可用，未选中的循环也被拒，已绑定的身份路由仍拒绝缺失用户凭证。

若改动还涉及下表中的边界，从源码仓根目录选择相应检查。

| 变更 | 命令 | 要核对的结果 |
| --- | --- | --- |
| Core 与扩展的源码隔离 | `python3 -B scripts/test-core-boundaries.py` | 检查测试实际覆盖到的文件、存储装配和边界断言。涉及运行时行为时补受影响入口的行为用例。 |
| 跨服务 proto 或生成绑定 | `bash scripts/generate-core-protocol.sh --check` | 两端生成物与契约一致。字段语义有变动时另用调用方测试验证兼容读取与失败返回。 |
| Web 消费方式 | `mise exec -- bun run typecheck:web` | 类型检查通过后，用受影响页面验证已启用、未启用或未获授权时的实际操作。 |

如果问题发生在支付回调重试，证据应包括原操作键、回执和隔离环境中的账本终态。一个编译成功的 RPC 客户端无法回答“同一笔钱入账了几次”。验证层次与已有证据的复用遵循 `docs/agent-workflows.md` 和 `docs/quality.md`；只运行能判定本次改动的检查。

## 写出评审结果

每项用一小段交代入口与版本、现状及证据、应该由谁决定、最小改法和复验结果。给出文件或符号、命令和实际观察。尚待复现的推测单列。若两个实现都需要保留，用一个具体输入说明它们各自保护的行为。按一次变更涉及的编辑点、失去用途的规则或消除的写入路径说明收益。

## 方法来源

- dependency-cruiser 的 `from`、`to`、`circular` 与 `reachable` 用于区分禁止方向、闭环与间接依赖。Web 问题可沿该思路追踪实际入口。[规则参考](https://github.com/sverweij/dependency-cruiser/blob/main/doc/rules-reference.md)
- ArchUnit 将模块关系写成可执行断言，并提供对已知违规建立基线的方法。它读取 JVM 字节码；本项目沿用同一思路，在现有 Go、Rust 或 Python 测试中表达规则。[ArchUnit 用户指南](https://www.archunit.org/userguide/html/000_Index.html)
- jscpd 按 token 匹配代码片段。阈值、排除项和扫描目录决定能发现哪些候选。用实际调用语义判断每条命中。[jscpd 官方项目](https://github.com/kucherenko/jscpd)
- 前端使用已有的 `apps/web/knip.config.ts`。从 `apps/web` 运行 `mise exec -- bun run knip`，先核对动态路由、独立入口和有意保留的公共导出，再删除无消费者的代码。[Knip 入口模型](https://knip.dev/explanations/how-knip-works)
- Rust 复用现有 `lint:core`，对照功能组合和调用方复核 Clippy 诊断。Go 的 deadcode 从可执行入口计算可达性，测试、构建标签和库的外部消费者会改变结论。[Clippy](https://doc.rust-lang.org/clippy/usage.html) · [Go deadcode](https://go.dev/blog/deadcode)

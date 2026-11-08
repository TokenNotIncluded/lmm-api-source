# 本地质量检查与独立验收

本页定义质量工具、业务验收与资源门槛；工具保存证据，不参与 Core 运行，也不授权发布。
当前候选的缺口和执行状态统一见[项目进度](progress.md)。旧轮次的结果只适用于其冻结输入。

> [!IMPORTANT]
> 日常开发按[工作指南](agent-workflows.md)选择必要检查。逐页清单、协议矩阵、资源门槛和独立复验
> 用于明确要求验收或项目收尾的任务，不是每次改动的完成条件；日常交付遵循[适度与停止](agent-workflows.md#适度与停止)。

## 先选所需层次

普通文档或技能正文使用 `python3 -B scripts/check-docs-brand.py --docs-only`。需要质量工具时直接选择一层，
不要按 quick → related → full 重复共享检查；工具不自动复用上次报告。集合以[quality-gate.py](../scripts/quality-gate.py)为准。

| 层次 | 实际范围 | 单项时限 |
| --- | --- | --- |
| `scan` | 变更、源码疑点、断言删除、敏感配置、债务期限与差异空白；不是业务测试 | 30 秒 |
| `quick` | scan、工具自测、源码边界及受影响组件格式 | 120 秒 |
| `related` | quick、固定 Core/路由/计费/Go 回归，按范围加入协议、Web 和数据库模块 | 600 秒 |
| `full` | 前述集合及全部组件、构建、lint、依赖、CLI、数据库和真实进程 | 1800 秒 |

`related` 在仅改文档时也运行固定回归。`full` 将独立资源与真实资金验收保留为 `not-run`，不能给出完整验收通过。
其 `scripts/mocklab.py selftest` 把组件报告与原始日志放在输出目录的 `mocklab-selftest/`，只证明 Mocklab 自身；
空集合、跳过、失败与环境阻塞保留非零结果。不得删减所选集合、断言或报告项，或以较小范围冒充完整验收。

## 执行

使用完整可信检出、固定工具链和工具要求的干净、已初始化 CLI 子模块。`--base` 必须是本次变更前已确认的
祖先提交，复用现有审查基点；不能在提交后改用新 HEAD 隐藏差异。输出使用仓库外的新目录，父目录须已存在。

```sh
BASE="${LMM_REVIEW_BASE:?Set the reviewed pre-change ancestor SHA}"
OUT="$(mktemp -d /tmp/lmm-quality.XXXXXX)"
mise exec -- python3 -B scripts/quality-gate.py quick --base "$BASE" --output "$OUT/quick"
```

将 `quick` 换为所需层次；完整目标直接执行 `full` 一次。等价入口是 `just quality`；工具改动使用
`just quality-self-tests`，聚合层已包含时不再重复。`--plan` 仅记录未执行计划，退出码为 2，不能计为通过。
执行前固定源码；期间变化使证据失效。独立失败继续收集，其前置构建失败只阻断依赖该产物的检查。

数据库必须是核实归属的本机一次性实例，显式设置 `LMM_QUALITY_DISPOSABLE=1`；所需变量为
`DATABASE_URL`、`LMM_STORE_TEST_DSN`、`PAYMENT_TEST_DSN`、`LMM_MK09_TEST_DSN`。支付库名为
`lmm_payments_test`，商店使用新的 `_test` 结尾库。localhost 可能是生产隧道；名称不能证明隔离。
工具不自动建库或读取 `.env`。Go 的 `go test ./...` 不跨嵌套模块，full 另检查 pgtest 和发现的 Go 模块。

支付存储期限回归另需 `PAYMENT_DEADLINE_TEST_DSN`，指向自有全新空库 `lmm_payments_deadline_test`。
在 `services/extensions/internal/modules/payments/pgtest` 执行 `mise exec -- go test -race -count=1 -run '^TestPostgresStorageDeadlines$' .`；
测试显式安装 schema，不删除或复用旧表，未配置时的 skip 不能计为通过。

[支付固定产物入口](../tests/payments/README.md)默认验证 Core 入账 COMMIT 后丢失 Go 本地回执；
`--case accepted-proof-expiry` 另验证实际等待至少 302 秒、双产品 OS 重启与首次入账恢复，期限为 480 秒。
过期原事件仍须被新接纳接口拒绝；恢复读取此前持久接纳的证明，核对原任务键、唯一回执、准确两腿账本和重放不变。
定位用 `markref context payments.proof-expiry`。这两种场景不替代真实 checkout/refund 或支付资源验收。
缺工具、实例或产物须报告受阻；外部供应商可模拟，内部身份、订单、路由和资金链不能替换为固定成功对象。

## 结果与复用

`report.json` 绑定源码、基点、工作区摘要、命令、目录、退出码、耗时和日志摘要。工作区摘要包含提交差异、
暂存/未暂存、新文件、权限和子模块状态；不完整证明外部缓存及子模块内部所有字节。环境覆盖项也须审查。
目录权限为 0700，日志/报告为 0600；分享前脱敏，不保存 DSN、Cookie、私钥或生产数据。
复用保留原源码及真实命令，不能给旧记录换上新提交；`scripts/local-release-tests.py` 的单命令记录不证明集合完整。

退出码：**0** 为所选层自动检查通过；**1** 为失败、到期或证据失效；**2** 为受阻、未执行或中断；**3** 为必须审查。
不得用 `|| true`、最后一行、空测试集或 skip/ignored 冒充通过。full 的开放 critical 债务阻断验收；独立资源与
真实资金项仍为 `not-run`，不能导入手写 JSON 改成通过。quick、构建和语法检查不等于业务验收。

## 疑点与例外

忽略、skip/only、空 catch、临时标记、测试环境分支和删除断言是审查线索，不自动等于缺陷；扫描也可能漏报。
保留新增与旧数量，逐项说明实际用途和证据，不为消除匹配删除有效测试或增加全局忽略。
`.quality/debt.json` 只记录确认事项；`baseline_revision` 是来源，不是豁免。开放事项到期前解决或用新证据续期，每窗最多 30 天。

例外使用 `kind=exception`，填写明确路径、原因、负责人、风险、问题、替代验证和退出条件，禁止根目录或通配符。
reviewer 初始为 pending 且不能是作者，解决须附证据。例外不取消命令、不改退出码，也不能豁免资金、权限、数据丢失或业务正确性。
质量工具、测试预期、依赖/锁文件、类型配置、工作流、忽略和阈值变化都须随业务代码审查。

## 测试与开发分工

完整验收与业务修复由不同人员或 agent 承担。换一个 agent 名称、读取作者摘要或重放手写结果不构成独立验证。

- **测试方**固定版本、产物、配置、角色和数据，独立确定预期、执行、记录和复验；可新增可复现测试，不修改被测业务后自验。
- **开发方**提交业务修复、原因、影响范围与自测；不能单方改变独立标准或把开发回归当验收结论。
- **整合方**保护并行文件归属，审查业务及测试差异，固定新提交并交回测试方，复验通过后才能关闭缺陷。

使用独立任务和输出目录，不在测试时修改同一输入；复用已有隔离检出或固定产物，共享源码由指定整合方写入。
账号、数据库和服务与生产隔离，凭据及原始日志不进 Git。普通结果在仓库外，长期恢复材料存入 `~/.local/state/lmm-api`。
结束后停止自建服务，核实归属再清理库、缓存和重复产物，保留失败证据及他人活跃材料。

缺陷记录须包含版本、页面 URL/协议方向、环境/角色、复现步骤、预期与实际、接口状态/脱敏日志、修复提交和复验结果。
状态区分发现问题、已修复待复验、复验通过、受阻、尚未覆盖；补丁返回、测试退出、推送、合并和部署也各自独立。
禁止删除失败用例、弱化断言、无依据 skip、吞异常、隐藏页面或用内部成功 mock 改绿。规格或断言确需修改时，
保留原失败及测试差异，由测试方核对新预期；资金、权限和数据完整性不降级。无法安排独立复验时只报告开发自测范围。

## 逐页验收

从待测路由、动态子页面注册和导航生成清单，覆盖深链接、刷新、页内切换、后台/详情/关键弹窗及各角色。
`/dashboard/flow` 与 `/dashboard/overview` 分别记录，不能用入口通过替代子页。

1. **渲染与响应**：桌面/移动端、明暗主题、浏览器异常、控制台错误、失败请求和响应结构；截图或 HTTP 200 不单独证明成功。
2. **数据状态**：加载、空/正常数据、未知字段、畸形响应、401/403、超时和 5xx；加载失败区别于空数据，未知金额不显示零。
3. **交互与权限**：筛选、分页、详情、提交/重试、拒绝、登出及账户切换缓存；角色、账户或时间范围变化不得复用错误数据。
4. **真实集成**：隔离环境连接实际 Core/Go，写入合成数据，核对最终状态与幂等；内部权限、订单、路由和账本保持真实。

mock 只用于契约一致的组件状态和故障注入，调试 fixture 不是生产兜底。分别记录组件/mock、真实浏览器、实际业务链路。
后端未实现、503、缺配置或未执行均不算页面通过；新发现错误须有复现用例，不能只补成功假数据。

## 协议转换验收

按待测源码建立“客户端协议 → 上游协议 → 输出模式”矩阵，逐项标明支持、明确拒绝、失败或未执行；
[relay 说明](../services/core/src/relay/README.md)提供协议边界。单个文本成功请求或历史记录不能代表完整矩阵。

- **请求**：模型别名、消息顺序、工具调用/结果关联、参数和鉴权；不支持字段明确拒绝，不静默丢弃。
- **普通响应**：文本、推理、工具 ID/参数、结束原因、usage 和供应商元数据；缺 usage 保持未知，核对语义而非仅 JSON 可解析。
- **流式响应**：真实 HTTP/SSE 分片、UTF-8/事件边界、增量工具参数、多工具关联、顺序/终态、截断、慢客户端及两种输出的一致性。
- **错误与取消**：4xx/5xx、连接/读取超时、断流、取消、大小/队列限制及敏感错误体；执行结果不确定时不得自动重放。
- **应用集成**：真实身份、选路、预留、结算和对账经过隔离库；核对拒绝无上游调用、取消回收、重复请求和账本终态。

分别报告解析/编码单测、真实 HTTP 转换、应用资金链和真实供应商兼容性。测试路由或计费 adapter 不替代应用验收；
前三者也不证明付费供应商已验证。真实付费调用须有授权和私有凭据，模拟脚本不证明生产准入。

## 独立业务与资源验收

独立审查者使用完整检出、固定工具链和一次性库，按所需层执行；完整目标只跑一次 full。
对比同一源码/配置的命令、日志和数据库终态，已有匹配证据可复用，但须标明证据复核而非独立执行。
模型、权限、计费、并发、重复请求、失败和流式取消经过真实应用，未接通链路不算完成。

资源比较固定机器或容器限制、release 构建、配置、数据规模和负载，记录程序摘要、CPU/内存硬限制、数据库版本/连接上限。
测启动就绪、预热后 RSS/PSS、线程、FD、连接，负载中的吞吐/延迟/内存增长、队列和在途任务，以及停止负载后的排空。
Core 单独与 Core+Go 分别验证；读回 1 CPU 配额及内存硬上限，CPU shares 和 Go 内存目标不能替代硬限制。
冻结持续时间、并发阶梯、业务比例和恢复门槛；数据库、生成器和受控上游是否计入预算须明确。
保留宿主/cgroup/进程/热点线程采样，区分容量拒绝、停机、路由不可用、计费失败与结算待确认。
不得用全量 503、删功能、调高阈值、丢任务或强制关池制造通过；资金终态、UNKNOWN 与原键重放始终核对。

### 合作式测试资源

[`test-resource-lease.py`](../scripts/test-resource-lease.py) 的 `run --root` 接受已有、绝对、当前用户所有且权限 0700 的私有目录，
每次生成独立 task/nonce、target 和追加回执。子命令以 `mise exec --` 开始；stdout/stderr 不转发，诊断写入 `LMM_TEST_OUTPUT`，凭据不进参数。
`--heavy` 使用已有私有 `/tmp/lmm-delegate` 中的共享 flock，不删锁或超时抢占；取消和写回执失败都等自有子孙进程退出后放锁，不承诺取消期限。
`--socket` 交付已监听的 `LMM_TEST_LISTEN_FD`，子进程直接接管；`--postgres` 仅创建新固定镜像库并核对物理身份、owner NOLOGIN 和低权限 runtime，不接受旧 DSN，凭据仅给子进程。
它不提供维护锁、沙箱或部署结论。回归用已有私有 `LMM_A09_TEST_ROOT` 执行 `python3 -B scripts/test_test_resource_lease.py`；
仅 `LMM_A09_TEST_POSTGRES=1` 执行真实新库项，否则是 SKIP。工具链仍须独立核对，wrapper 环境不是版本证明。

### 固定产物与 Core 资源入口

[`verify_model_resources.py`](../scripts/verify_model_resources.py) 要求固定 Rust、本地固定 PostgreSQL 镜像、用户级 systemd 和可读回的 cgroup v2 硬限制。
它不安装工具或连接外部库；资源用例通过 `resource-acceptance` feature 和显式 ignored 选择，与普通模型流程分开。
`RESOURCE_OUTPUT` 为新的私有绝对输出目录，路径无符号链接；`BUILD_RESOURCE`、`BUILD_REQUESTS` 复用已有重型构建租约。
`CORE_BIN`、`SOURCE_BINDING`、`BUILD_RECEIPT` 来自同一成功且无源码漂移的 release 构建。manifest 的 `files`
须恰好覆盖入口的 `PRODUCT_TREES`/`PRODUCT_FILES` 并映射到 SHA-256，回执绑定 manifest 与程序摘要；格式见[模型流程入口](../scripts/verify_model_process.py)。
`PROCESS_ROOT` 须为本轮独占的空目录，权限 0700、绝对路径不超过 35 字节。按真实 prepare 输出设置 `PREPARED_SHA256`：

```sh
python3 -B scripts/verify_model_resources.py prepare \
  --output "$RESOURCE_OUTPUT" --resource "$BUILD_RESOURCE" --requests "$BUILD_REQUESTS" \
  --core-binary "$CORE_BIN" --source-binding "$SOURCE_BINDING" --build-receipt "$BUILD_RECEIPT"
python3 -B scripts/verify_model_resources.py run --output "$RESOURCE_OUTPUT" \
  --process-root "$PROCESS_ROOT" --prepared-sha256 "$PREPARED_SHA256"
```

prepare 成功不是验收通过。Core 预算为 **1 CPU / 256 MiB**；数据库、生成器与模拟上游在预算外，不能合称整机通过。
默认持续负载不覆盖真实 socket 背压、Go 联合预算、真实供应商或旧架构对照。每次 run 使用新的空进程目录。

### 持续负载与联合门槛

540 秒分为预热 60、稳态 300、过载 60、有流量恢复 60、空闲排空 60 秒，分别留证据。
正常阶段 **5 RPS、80% JSON / 20% SSE、完整成功率 ≥99%、P95 ≤1 秒、P99 ≤2 秒**；过载须有限拒绝且随后恢复。
完整合成 Chat 响应校验请求/回执 ID、结构、内容、结束原因和 usage；SSE 核对实际 content、起始角色、唯一 stop、
唯一正确 usage、最后 DONE 及顺序。坏 JSON、error 帧、缺失/重复终态或终态后数据均失败，metadata 字符串不算答案。
判定器反例见 [`model_resource_response`](../services/core/tests/model_resource_response.rs)，定位 `markref context model.response-validation`。

重启积压分类独立计时：恢复器每 5 秒最多 64 条，按实际剩余量推导有限期限且持续取得进展，保留全部 UNKNOWN 预留款。
就绪不表示积压清零，也不能把分类耗时混算为 120 秒流量恢复。失败先确认自有 Core unit 停止，再调查并保留库与原回执。

Core+Go 共享 **1 CPU / 512 MiB**，入口为 [`tests/commerce/verify_process.py`](../tests/commerce/verify_process.py) 的
`--shared-resources --mixed-load`，提供 Core、Go、测试程序三组固定产物及 SHA256；[联合入口说明](../tests/commerce/README.md)列出参数。
模型执行同一 540 秒门槛，并在有流量阶段每秒完成一笔购买、重复购买、并发支付、履约、结算、退款和重试链路。
商店必须达到真实退款终态，P95 ≤1 秒/P99 ≤2 秒，整笔共用计划提交起 2 秒绝对期限；202 或拒绝不是成功。
逐笔核对原买卖方、金额、唯一 Core 操作/journal、Go 回执、库存及余额。资源组预检入口是
[`verify_resource_group.py`](../tests/commerce/verify_resource_group.py)，它只验证资源工具，不能替代产品联合负载。

### 真实 socket 与 HTTP/2

Core 的 `run --case socket-backpressure`、`socket-pressure`、`socket-h2` 分别为短回归、持续停读和共享 HTTP/2 流隔离。
保留产品输出/队列、5 秒 socket 停滞保护和 1 CPU / 256 MiB；仅 fixture 使用 30 秒请求窗口、60 秒租约和 30000ms 路由。
正常大流、背压后续读和恢复请求各结算 49 credits、返还 91；停读超时/主动断开保留 140、capture 为零，原键重放 409 且不再调用上游。

持续停读固定四个 60 秒阶段：正常 5 RPS、叠加每秒两个停读请求、保留全部 120 个客户端的有流量恢复、释放后的空闲排空。
前三段计划 900 个正常请求，沿用 ≥99%、P95 ≤1 秒/P99 ≤2 秒；逐连接发送队列、persist 零窗口探测和 checkpoint 停滞共同证明真实背压。
恢复结束前残留非零发送队列必须清零，客户端仍保留原 FD/device/inode；排空检查 socket/进程内存、FD、数据库连接和 cgroup 事件。
不能提前关闭客户端掩盖失败。合法 JSON 转义可放大线上字节，但不得提高解码输出限额。
HTTP/2 在同一已核对 TCP 上保留未读流，完成 12 次正常请求及取消后一次请求，并核账/重放；它证明流隔离，不等同 TCP 零窗口实验。

联合停读使用 `--shared-resources --mixed-slow-sockets` 和三组固定产物，与 `--mixed-load` 互斥；同样为 240 秒、120 个保留客户端，
前三段 5 RPS 模型与 1 RPS 完整商店退款，共计划 900/180 笔，产品共享 1 CPU / 512 MiB，其他进程预算另列。
定位：`markref context model.socket-pressure`、`markref context commerce.socket-pressure`。Core 单独通过不能代替联合验收。

### 公共连接与请求头边界

[`public_server`](../services/core/src/public_server.rs) 在 HTTP 解析前最多接纳 64 个公共 TCP，另有一个 accept 后待判定的临时 socket；
HTTP/2 按连接计数。超额立即重置且不创建处理任务；首个请求头自 accept 起有 10 秒绝对期限，覆盖静默、慢 HTTP/1 和不完整 HTTP/2 前言。
后续 HTTP/1 使用 Hyper 原生 10 秒请求头期限；SSE、已解析 HTTP/2 和升级连接不新增总时限，升级名额直到实际 IO 释放。
此合同不覆盖已建立 HTTP/2 上每个新流的请求头期限。无法设置重置关闭时返回错误、停止监听并排空，不能静默降级关闭方式。

入口 [`verify_public_ingress.py`](../scripts/verify_public_ingress.py) 支持 `--mode unit|red|green`，要求固定 Core/测试程序及各自 SHA256、
`--source-binding`、`--build-receipt`，以及全新空私有 `--output`、`--process-root`。完整参数用 `python3 -B scripts/verify_public_ingress.py --help`。
只新建自有隔离库，失败保留 fixture；四类原始连接各 128 次尝试，保留客户端 FD/device/inode 到 20 秒后核回收，
同时验证正常模型资金、健康 HTTP/2 与资源清理。Core 为 1 CPU / 256 MiB，生成器和数据库预算另列；定位 `runtime.connection-admission`。

### 历史证据与旧架构对照

历次成功数字、固定输入及失败保留在 [0eae39f 的旧质量记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/quality.md)。
其中旧 socket 产物未完成 240 秒并触发 OOM，公共连接旧产物超期残留等失败不能覆盖或改算成功；重置设置失败分支当时仅有源码审查，未做实际故障注入。
后期联合 240/540 秒轮次是**开发者执行，独立测试者从冻结输入、原始采样和回执重建核对**，不是独立重新运行，更不是当前候选已验收。

最终交付须证明相对旧架构的实质性能改善、更低内存和更好稳定性；保留旧版官方产物及双方源码/构建/配置证据。
双方使用相同 CPU/内存、数据库规模、计费语义、预热与固定负载。
分别报告成功/拒绝/超时、吞吐、P50/P95/P99、常驻/峰值 RSS/PSS、长期增长和故障恢复；核对普通退出、数据库重连和扩展重启后的资金与资源。
正式对照使用优化构建，保留原始采样及前后差异；未实测不写性能改善或内存百分比，不能用 429/503、行数或测试数量冒充业务结果。
工具不能认证审查者身份或阻止作者改脚本；远端保护、独立审查、权限与任务成本另行授权，文档不修改工作流。
`just check` 是日常开发集合，不是本页完整验收入口；当前状态回到[项目进度](progress.md)。

# Rust 核心与 Go 扩展通信

使用 Protobuf 定义二进制消息，gRPC 发送请求和接收结果。同机进程通过私有 Unix socket 连接。这不是共享内存或消息代理。持久事件由 Core 数据库中的事件机制负责；助手可通过专用 RPC 观察已授权 worker 的输出，公共模型流量不经 Go 转发。

Go 唯一源码目录为 `services/extensions`。核心只有一个身份与账号权威来源，不从 Go 的旧用户或余额表回退。普通启动不建库或自动升级；新装与既有库升级使用显式离线命令，原生部署命令见 [部署指南](../deployment/native/README.md)。

## 方向与权限

Go 模块 → 复用的 gRPC 客户端 → 私有 socket → Rust → 按接口使用只读查询或 Core 管理的业务事务。

Rust 不反向等待 Go，核心 HTTP 身份检查不经过扩展。Go 不持有核心数据库地址、密码或数据卷。用户身份不能由 Go 上传的角色、用户 ID 或付款账号代替。

基础只读合同为 `contracts/proto/lmm/core/v1/control.proto`，服务为 `lmm.core.v1.CoreControl`：

| 方法 | 作用 | 用户凭证 |
| --- | --- | --- |
| `Capabilities` | 查询协议主版本和已配置功能。 | 不需要，但仍需服务凭证。 |
| `Authorize` | 查询真实身份、Key 归属和有序付款候选。 | 会话或 API Key。 |
| `ListTeams` | 按 `after_id` 分页，最多 100 个团队。 | 必须是会话。 |
| `AuthorizeRemote` | 验证个人远程控制权限，区分原生会话和 OAuth grant 来源。 | 专用远程授权凭证；不是通用资金授权。 |

支付、账本、商店、助手和事件使用同目录其他 proto 定义的独立服务；上述基础方法不代表全部 RPC。

`identity_available` 只表示身份存储已配置，不保证数据库此刻可用，也不保证模型就绪。付款候选是权限快照，不是已预留余额。资金写操作必须在自己的事务中重新校验。

`Account.id` 仍是该 `kind` 下的公开用户或团队 ID，不因增加内部 `accounts.id` 而改变含义。新增需要内部账号标识的接口应使用明确的新字段，不能暗中改写已有字段语义。

内部调用分别携带服务凭证、协议主版本和用户凭证：`authorization`、`x-lmm-protocol`、`x-lmm-user-credential`。Rust 拒绝重复、缺失、无效凭证以及不支持的协议主版本。Go 客户端不允许外部元数据覆盖内部服务凭证。凭证不进入 Protobuf 消息正文、响应或日志，授权结果不缓存。

## 业务 RPC 与契约边界

以下服务见同目录 Protobuf 契约，实际挂载由 [`internal_rpc.rs`](../services/core/src/internal_rpc.rs) 的服务装配决定：

| 服务 / 契约 | 职责与边界 |
| --- | --- |
| `CoreEvents` / [`events.proto`](../contracts/proto/lmm/core/v1/events.proto) | `Negotiate`、`Pull`、`Acknowledge`、`Retry`；按服务隔离的持久事件订阅、租约、确认和重试，不是任意消息发布接口。 |
| `CoreCommerce` / [`commerce.proto`](../contracts/proto/lmm/core/v1/commerce.proto) | 商店固定命令执行、结果查询、原授权交付结算与丢失 ACK 对账；不能任意指定账目或铸造余额。 |
| `CorePayments` / [`payments.proto`](../contracts/proto/lmm/core/v1/payments.proto) | Core 验证支付证据、冻结充值意图、入账和退款状态；Go 上传的付款状态不能替代 Core 验证。 |
| `CoreSubscriptionPayments` / [`payments.proto`](../contracts/proto/lmm/core/v1/payments.proto) | Pancake、Stripe、EPay 订阅订单、支付证据与权益发放；订阅权益不等于钱包现金。 |
| `CoreCheckinLedger` / [`checkin_ledger.proto`](../contracts/proto/lmm/core/v1/checkin_ledger.proto) | `Inspect`、`Post`；受所有者上限约束的签到赠额，不开放任意账户或余额替换。 |
| `CoreDirectoryAds` / [`directory_ads.proto`](../contracts/proto/lmm/core/v1/directory_ads.proto) | 广告报价、付款、退款、查询和对账；使用独立广告服务身份。 |
| `CoreAssistantModel` / [`assistant.proto`](../contracts/proto/lmm/core/v1/assistant.proto) | 会话、turn grant、文本预留、操作查询、输出观察、agent turn 与工具确认；Core 保留模型调用和资金权威。输出观察不会启动或重放付费请求。 |

`CoreControl` 和 `CoreEvents` 在内部监听器中注册；上表其余业务服务按配置可选挂载，各方法还需对应权限、能力协商和已安装的持久执行路径。契约存在、服务挂载、能力发布、模块启用和独立业务验收是不同状态。

[`commands.proto`](../contracts/proto/lmm/core/v1/commands.proto) 的通用身份、账本、计费、路由命令和 [`ledger.proto`](../contracts/proto/lmm/core/v1/ledger.proto) 的 `CoreLedger` 保留集成契约；当前内部监听器没有注册这些通用服务。不要把它们当作可调用的任意资金写入口。

## 有界请求与失败处理

单条消息上限 64 KiB。Rust 最多接入 8 条内部连接，每连接最多 8 个请求流，实际查询还受全局 8 个名额限制。Go 客户端最多保留 32 个正在调用的请求；Go 模块入口另外限制每模块 8 个请求。超限立即失败，不无限排队。HTTP/2 流量窗口不是持久业务队列。

内部服务器默认请求期限为 2 秒；Go 客户端按方法设置有界期限（如支付证据接收为 4 秒），更短的调用期限仍有效。助手输出观察另有有界流生命周期，普通调用期限不代表整个流的时长。取消和超时释放对应名额；未知资金结果必须按原操作键和业务对账契约恢复，不能创建新扣款或凭超时补偿。

CoreControl 的只读查询池只使用 2 条连接，取连接最多等待 400 ms，语句最多执行 1500 ms，并设为只读。它不占用核心 HTTP 身份池的连接名额，但仍共享数据库和宿主机资源，不是完整故障隔离。

Go 将 gRPC 失败映射为明确 HTTP 状态，不返回底层 SQL 或密钥。核心离线时 Go 仍可启动；用户查询失败关闭，核心恢复后复用客户端重新连接。进程健康不等于业务就绪。

## 同机私有连接

没有内部 TCP 监听或明文跨主机回退。跨机器部署需要另外设计双向 TLS 与服务身份，不能把 socket 通过公开转发工具暴露。

核心是 socket 目录唯一写入方。默认目录 0700、socket 0600，两端使用相同专用 UID；共享 Unix 组访问必须显式配置并匹配服务管理器权限。Go 只读挂载目录，可以连接，但不能创建、替换或删除文件。核心启动时使用独占锁，避免覆盖其他实例持有的监听文件。

`compose.rpc-core.yml` 和 `compose.rpc-extensions.yml` 分别叠加到独立核心与扩展项目。更新扩展只操作扩展项目，不删除核心或外部 socket 卷。宿主文件用 `LMM_CORE_RPC_TOKEN_SECRET_FILE` 配置，卷名可用 `LMM_CORE_RPC_VOLUME` 配置。

当前进程使用绝对路径 TOML 配置：Core 的 `[rpc].socket` / `token_file`，Go 的 `[core].socket` / `identity_token_file`，见 [Core 示例](../config/core.example.toml) 和 [扩展示例](../config/extensions.example.toml)。`LMM_CORE_RPC_SOCKET` / `LMM_CORE_RPC_TOKEN_FILE` 属于显式 `--legacy-env` 兼容入口，不能与 TOML 配置混用。凭证当前在启动时读取，没有热轮换；更换内部凭证需要协调两端，不能只改 Go 文件就假设 Rust 已更新。

## 接入 Go 模块

模块只收到自己需要的接口和依赖，不获得全局数据库或任意 SQL 通道。`internal/modules/identity` 使用一个仅含三个只读方法的接口，由 `internal/coreclient.Client` 实现。

```text
GET /extensions/v1/identity/capabilities
GET /extensions/v1/identity/self
GET /extensions/v1/identity/teams?after_id=0
```

宿主先检查自己的 `extensions-token`，然后在进入模块前移除该 `Authorization` 请求头。需要用户身份的操作另外读取 `X-LMM-User-Credential` 并交给 Rust 验证。不能把宿主服务密钥发给浏览器，也不能将这些内部接口当成新的公共登录入口。

返回使用 Protobuf JSON，64 位 ID 为字符串。TOML 的 `[host].modules` 控制显式启用列表；`["none"]` 关闭全部模块。`LMM_EXTENSION_MODULES` 仅属于 `--legacy-env` 兼容入口。默认只在 RPC 已配置时启用 identity。未知或重复名称使启动失败，新模块不会自动对所有部署启用。

这些模块是可信的编译期 Go 代码，不是不可信插件沙箱。需要进程级隔离时，使用独立扩展容器。当前 [`composition/registry.go`](../services/extensions/internal/composition/registry.go) 注册 12 个模块：`checkins`、`identity`、`store`、`support`、`assistant`、`remote-control`、`payments`、`toolmarket`、`hero-sms`、`operations`、`community`、`acquisition`。`support` 必须同时显式选择 `store`；`operations`、`community`、`acquisition` 的 Web 适配器由进程入口装配。注册不等于全部启用，也不证明业务验收或生产部署完成。

## 生成与验证

Rust 构建时从同一 `.proto` 生成绑定。Go 的生成文件提交入库，普通构建不需要 protoc。以下命令从仓库根目录运行，使用 `mise.toml` 固定的工具链（当前 Rust 1.100.0、Go 1.27.2）；`services/extensions/go.mod` 的 `go 1.26.0` 是最低语言版本，不是当前开发工具链 pin。协议生成另需系统提供 `protoc`。

```sh
mise exec -- bash scripts/generate-core-protocol.sh
mise exec -- bash scripts/generate-core-protocol.sh --check
(cd services/extensions && mise exec -- go mod verify && mise exec -- go vet ./... && mise exec -- go test -race ./... -count=1)
# DATABASE_URL 必须指向可创建临时测试数据库的独立 PostgreSQL。
(cd services/core && mise exec -- cargo fmt --all --check && mise exec -- cargo clippy --locked --all-targets -- -D warnings && mise exec -- cargo test --locked --all-targets)
python3 -B scripts/test-core-boundaries.py
mise exec -- just test-docker \
  "${LMM_DOCKER_TEST_OUTPUT:?Set a new empty private absolute output directory}"
```

不得复用已有字段编号，不能悄悄改变类型、枚举或 RPC 名称。增加字段不能让旧调用者无故失效；破坏性改动应使用新主版本。契约测试覆盖已有字段、大整数、未知字段和损坏消息。

`.github/workflows/core-protocol.yml` 按当前 main 的路径过滤触发，并提供 workflow_call 和手动入口；它检查生成一致性、编译、并发测试、真实数据库和 Docker 通信。Docker 使用随机临时项目验证凭证隔离、团队查询、扩展重建、核心重启重连、数据库故障恢复和即时撤销。

事件必须与核心变更一起持久化，再独立交付，并处理重复、确认、重试和积压上限；当前 `CoreEvents` 已提供相应订阅协议。资金写入通过专用业务服务受控执行，不能再概括为“未来实现”。核心模型路径不应新增同步 Go 回调；模型 API 是否可用取决于实际模型配置、存储和运行状态，未配置或未就绪时仍须明确失败，不能把所有模型请求一概描述为固定返回 503。本页是接口说明，不替代 [当前进度](progress.md) 中的独立验收证据。

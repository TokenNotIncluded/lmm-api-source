# Core 与 Extensions 架构

本文维护程序职责、数据权威和安装边界；当前功能、验收和任务只见[项目进度](progress.md)。
仓库关系与工作主线见[开发指南](development.md#仓库与主线)。

## 三个程序的职责

| 程序 | 职责 | 权威边界 |
| --- | --- | --- |
| `lmm-core`（Rust） | 身份、权限、账户、账本、预算、订阅、模型路由和转发 | 身份与资金的唯一权威，启动不等待 Go |
| `lmm-extensions`（Go） | 商店、工具、助手、运营、支付供应商及公开协议 | 持有扩展业务记录，经窄 RPC 请求 Core 授权与资金操作 |
| `lmm` 客户端 | 连接服务并执行用户操作 | 最终权限和扣费由服务端决定 |

`apps/web` 独立构建静态文件，由 Web 服务器提供；安装、管理和升级通过 `lmm-core`
显式子命令完成，不另设业务权威或管理进程。

| 目录 | 内容 |
| --- | --- |
| `services/core/` | 单个 Cargo 项目的 Rust Core |
| `services/extensions/` | Go 宿主与 `internal/modules/` 业务模块 |
| `apps/cli/`、`apps/web/` | 客户端 Git 子模块、静态 Web |
| `contracts/`、`config/` | 跨服务协议、配置模板 |
| `deployment/`、`packaging/` | 部署与分发定义 |
| `scripts/`、`docs/` | 开发验证工具与使用运维文档 |

Core、Extensions 分别构建、打包和更新；Go 模块仅在需要独立部署时拆服务。
目录、并发上限和内存配额不提供进程隔离，不可信代码须独立进程；容器隔离也不覆盖
单主机故障。

## 身份与资金由 Core 决定

用户、团队、经济账户、操作人和付款方分别建模，平台等级、管理权限和团队角色
分别检查。团队 Key 使用团队资金，个人 Key 默认使用个人资金；Go 或浏览器提交的
账户、角色、余额、折扣和付款方信息不得作为授权事实。

收费固定操作人、付款方、价格版本、预算周期和资金分项，权限、预留、预算和事件
使用一致事务边界；上游调用和流式响应不持有长数据库事务。重试、结算和退款引用
原操作，不重新选择付款方或价格。

模型流式输出先保存证据再发送；缺最终用量时保留预留款并等待恢复，不按零用量结算。
缺授权、计费依赖不可用或计价维度未受支持时明确拒绝。合同见[模型与计费](model-billing.md)。
供应商成功是外部事件，Core 验证证据并生成账本回执后才确认入账；Go 保存订单、
回调和恢复任务，不直接改 Core 余额。详见[支付网关](payment-gateways.md)和[钱包历史](wallet-history.md)。

## Go 的存储与 RPC 边界

持久化扩展使用各自的独立数据库、受限角色和窄存储接口，不访问 Core 库，不向模块
传入共享全局数据库对象。身份、购买、支付及模型委托通过对应 RPC；宿主凭证不
替代用户凭证或扩展 RPC 权限。模块由配置显式装配，缺 Core 权限、存储或适配器时
拒绝受影响操作，不能免费放行。

公开协议读取 TOML 指定的只读文档快照，无需服务凭证或 Core。配置见
[Core 模板](../config/core.example.toml)、[Extensions 模板](../config/extensions.example.toml)，
分库授权见 [PostgreSQL 指南](../deployment/postgres/README.md)，协议见 [`contracts/`](../contracts/)。

## 独立启动与资源

Core 无需 Go、前端或扩展数据库，身份和账户仍依赖 PostgreSQL。模型与可选业务
默认关闭；`/health/ready` 可在已配置组件通过时返回 200，同时声明模型或支付关闭；
模型关闭时 `/health/model-ready` 返回 503。就绪不验证外部上游或整站业务。
独立配置用于隔离环境，不能直接替换生产网关或连接生产库。

低内存配置面向 1 核 / 1 GiB 节点，下表是预算，未宣称实测 RSS 或吞吐量：

| 项目 | 默认 | 配置或范围 |
| --- | --- | --- |
| Tokio I/O 工作者线程 | 1 | `server.worker_threads=1..4` |
| 数据库连接池上限 | 2 | `database.max_connections=1..8` |
| 连接/获取连接等待 | 3 秒 | `database.acquire_timeout_seconds=1..30` |
| `check-db` 连接后的完整只读观察期限 | 15 秒 | `database.check_timeout_seconds=1..120` |
| 同时密码哈希任务 | 1 | `identity.password_slots=1..2` |
| 已接纳请求与后台完成任务 | 16 | `server.max_inflight=1..64` |
| Tokio 阻塞线程 | 4 | 固定 |
| 身份与可选 RPC 池空闲回收 | 30 秒 | 固定 |
| 独立 Core 容器内存上限 | 384 MiB | [compose.core.yml](../deployment/docker/compose.core.yml)，可显式覆盖 |

参数由 [`config.rs`](../services/core/src/config.rs)、[`main.rs`](../services/core/src/main.rs)
和 [配置模板](../config/core.example.toml)定义。Argon2id 保留 19,456 KiB、两次迭代及
原校验规则。连接等待和 `check-db` 总期限独立于 SQL 的 5 秒、事务空闲的 10 秒及
资金操作期限；TOML 不接受环境覆盖。旧测试模式使用
`LMM_CORE_DB_ACQUIRE_TIMEOUT_SECONDS`；`LMM_CORE_DB_CHECK_TIMEOUT_SECONDS` 是内部映射，
`check-db` 始终要求显式 TOML，禁止 `--legacy-env`。

未配置 RPC 时不创建其监听器和额外连接池。需要共享 socket 时再配置
`compose.rpc-core.yml`、`rpc.shared_group_id`、`lmm-rpc` 账户、IPC 目录及 token 权限；
Core 私密目录和持久状态不向扩展共享。可恢复 API Key 使用独立持久的 32 字节
`identity.api_key_encryption_key_file`，不借用 Web refresh key；当前无 keyring 迁移，
直接更换主密钥会使旧密文揭示失败。

[原生预览包](../packaging/linux/README.md)通过 systemd/OpenRC 传入
`--config /etc/lmm.conf.d/core.toml`，不自动初始化、授权或启动。操作者准备服务账户、
凭据和限制，日志默认进入 journal。配置和 owner/runtime 分离见[开发指南](development.md#核心与数据库)。

资源调优前，在已完成授权及启动验证的隔离机器测量空闲 5 分钟 RSS/PSS、8/16 并发、
长流、失败登录、断库恢复、回收、OOM 和端到端延迟；Core、PostgreSQL、进程内存和
含页缓存的 cgroup 内存分开记录。容器镜像缩小不代表常驻内存降低：

```sh
mise exec -- docker --host unix:///var/run/docker.sock stats --no-stream "$LMM_LOCAL_CORE_CONTAINER"
mise exec -- docker --host unix:///var/run/docker.sock inspect --format '{{.State.OOMKilled}} {{.State.RestartCount}}' "$LMM_LOCAL_CORE_CONTAINER"
```

内存不足时拒绝额外请求，不削弱密码哈希、权限、预留、结算或持久化。不要把所有
模块和数据库挤进同一台小主机后套用这些预算；独立部署也不保证单节点更新不断流。
历史资源与身份迁移结果见[归档](archive.md)，不作为当前候选的容量或完成状态。

## 安装、权限与结构升级

结构合同以待运行二进制的 `schema-manifest` 和 [`core_schema.rs`](../services/core/src/core_schema.rs)
为准。指纹按编译时执行的完整 SQL 字节计算，可能包含多个片段，不能只散列单文件。
结构安装建立数据合同，业务仍须配置、权限和依赖就绪。

| 操作 | 入口与约束 |
| --- | --- |
| 查看合同 | `schema-manifest` 不连接数据库；`check-db` 用显式配置只读检查目标 |
| 安装空库 | `init-db` 使用独立 installer/owner，只面向新的空 Core 库 |
| 正式首位管理员 | `create-first-owner` 离线读取私有 stdin JSON，要求显式配置和符合条件的新库 |
| 开发身份引导 | `bootstrap-user` 仅供开发，一次性凭据写私有文件 |
| 升级原生库 | 使用匹配前置合同的离线过程、执行器和绑定请求 |
| 普通启动 | 受限运行账户校验组件合同、实时 catalog 指纹和权限 |

普通启动不创建角色、授权、执行 DDL 或自动升级。命令语义以
[`main.rs`](../services/core/src/main.rs)和二进制帮助为准；安装顺序见
[Arch Linux 指南](archlinux-install-update.md)，路径见[文件布局](filesystem-layout.md)。

离线升级绑定 PostgreSQL 前置结构、目标实例、角色、catalog、行和序列，保留原
请求身份恢复。历史升级使用匹配的已保留执行器；新二进制不自动适用于所有旧库。
更新源码、重启服务不代替升级，也不得清空或重新初始化已有原生库来绕过它。

## 旧数据迁移与证据

从只读备份恢复的隔离副本读旧数据，写入另一个隔离目标库。
[`migrate-legacy-snapshot.py`](../scripts/migrate-legacy-snapshot.py)默认预览；身份迁移
只处理账号与受控密码验证信息，API Key、会话、余额、订阅和历史各有受控导入、
对账过程，无法确认的权限及登录方式保持关闭。

实际写入绑定来源、目标和结构凭据，验证原子事务、原键重放及冲突拒绝；生产数据、
映射、预览和连接信息不进仓库。`--source-rights-preview` 只核对冻结经济权利，
不生成导入授权或代表资金迁移完成，见[历史余额导入](legacy-financial-opening.md#先定位待处理的经济权利)。

早期身份、公告、HTTP 和 Web 批次见[固定历史版本](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/core-migration.md)。
新候选绑定自己的源码、命令、退出码和日志，区分单元、模拟渠道、数据库、进程、
浏览器和供应商证据；U1–U15 结构整理及其未完成验收保留在[归档](archive.md)。

## 来源与许可

LMM API 是 [QuantumNous/new-api](https://github.com/QuantumNous/new-api) 的维护分支，保留
[AGPLv3 LICENSE](../LICENSE)、[NOTICE](../NOTICE)及[第三方许可](../THIRD-PARTY-LICENSES.md)。
界面保留 NOTICE 要求的可见原项目链接与署名，前端源码保留上游头部及单独的
`Copyright (C) 2026 LIghtJUNction` 修改声明。

历史 Go 快照为 `ba2e9287bb7a8002116c03daa4c457a330054871`（2026-08-29），
已审查上游提交为 `7aa3531ef4c247ad4891c06cc8ed9d0ffb73fecc`（2026-10-08）；
选择、延期和拒绝移植的原因见[固定 fork 记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/8e92080aa3e9cb180103e73cb42e78b87621a31e/docs/fork.md)。
当前扩展模块路径以 [`go.mod`](../services/extensions/go.mod) 为准。后续更新固定并审查
来源，保留品牌补丁、兼容标识及本地资金合同，按受影响行为验证。

TypeSafe Jev 的历史 Go 原生实现来自 Apache-2.0 的 `QuantumNous/new-api-plugins`
`97a16e9a8d98b73ffb76e1c5a00b4d0f855f4d0a`（TypeSafe 1.0.0），未引入 JS task host。
当前 Rust System One 协议、配置和验收边界见 [Jev 原生接入](text-request-and-native-models.md#system-one)。

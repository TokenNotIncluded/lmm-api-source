# Core 与 Extensions 架构

本文说明程序职责、数据权威和安装边界。当前功能差距、验收和交付任务统一见
[项目进度](progress.md)；本文不维护组件数量、测试累计或迁移批次状态。
源码位于 `TokenNotIncluded/lmm-api-source`，父仓库 `TokenNotIncluded/lmm-api` 保存产品配方。

## 三个程序的职责

| 程序 | 职责 | 权威边界 |
| --- | --- | --- |
| `lmm-core`（Rust） | 身份、权限、账户、账本、预算、订阅、模型路由和转发 | 身份与资金的唯一权威；启动不等待 Go 扩展 |
| `lmm-extensions`（Go） | 商店、工具、助手、运营、支付供应商适配及公开协议 | 持有扩展业务记录，经窄 RPC 请求 Core 授权与资金操作 |
| `lmm` 客户端 | 连接服务并执行用户操作 | 最终权限和扣费由服务端决定 |

`apps/web` 是独立构建的静态前端，由 Web 服务器提供，不增加一个业务权威进程。
安装、管理和升级使用 `lmm-core` 的显式子命令，不另设管理程序。

| 目录 | 内容 |
| --- | --- |
| `services/core/` | 单个 Cargo 项目的 Rust Core |
| `services/extensions/` | Go 宿主及 `internal/modules/` 下的业务模块 |
| `apps/cli/`、`apps/web/` | 独立客户端仓库的 Git 子模块、静态 Web 应用 |
| `contracts/`、`config/` | 跨服务协议、配置模板 |
| `deployment/`、`packaging/` | 部署定义、分发包定义 |
| `scripts/`、`docs/` | 开发与验证工具、使用及运维文档 |

Core 与 Extensions 分别构建、打包和更新。Go 模块仅在需要独立部署时拆成服务；
目录拆分、并发上限和内存配额本身不提供进程隔离，不可信代码须放在独立进程中。
单主机故障也不在容器隔离的保证范围内。

## 身份与资金由 Core 决定

用户、团队、经济账户、操作人和付款方分别建模。平台等级、平台管理权限与团队角色
分别检查；团队 Key 使用所属团队资金，个人 Key 默认使用个人资金。
Go 或浏览器提交的账户、角色、余额、折扣和付款方信息不能成为授权事实。

一次收费固定操作人、付款方、价格版本、预算周期和资金分项。权限检查、预留、预算
与事件写入遵守一致的事务边界；调用上游和发送流式响应不持有长数据库事务。
重试、结算与退款引用原操作，不重新选择付款方或价格。

模型流式输出须先保存对应证据，再向客户端发送。缺少最终用量时保留预留款并进入
待恢复状态，不能按零用量结算。未支持的计价维度、缺失授权或不可用的计费依赖必须
明确拒绝。路由、定价和结算合同见[模型与计费](model-billing.md)。

支付供应商返回成功只是外部事件；Core 验证原始证据并生成账本回执后，钱包才能
确认入账。Go 保存订单、回调收件记录和恢复任务，不能直接修改 Core 余额。
具体配置及退款边界见[支付网关](payment-gateways.md)，查询合同见[钱包历史](wallet-history.md)。

## Go 的存储与 RPC 边界

持久化扩展使用各自约定的独立数据库、受限运行角色和窄存储接口，不访问 Core
数据库，也不向模块传递一个共享的全局数据库对象。跨边界的身份、购买、支付与
模型委托通过对应 RPC 完成；宿主凭证不能代替用户凭证或任意扩展 RPC 的权限。

模块必须由宿主配置显式装配。缺少必需的 Core 权限、数据存储或适配器时，受影响
操作返回不可用或拒绝，不能免费放行。公开协议读取使用 TOML 指定的只读文档快照，
不需要服务凭证，也不通过 Core 读取。

配置入口是 [Core 模板](../config/core.example.toml) 与
[Extensions 模板](../config/extensions.example.toml)。数据库角色与分库步骤见
[PostgreSQL 配置](../deployment/postgres/README.md)，跨服务协议位于
[`contracts/`](../contracts/)。

## 独立启动与健康检查

Core 可以在没有 Go 进程、Go 镜像和 Go 数据库的情况下运行；身份和账户功能仍需要
PostgreSQL。模型入口默认关闭，关闭时返回 503。部署尺寸和资源配置见
[核心独立部署](core-standalone.md)。

`/health/ready` 表示已配置组件通过检查，允许同时声明模型或支付关闭。
`/health/model-ready` 在模型关闭时返回 503。健康响应不能证明外部上游可用，
也不能替代整站验收。

## 安装、权限与结构升级

结构合同以待运行二进制的 `schema-manifest` 和
[`core_schema.rs`](../services/core/src/core_schema.rs) 为准。组件指纹按编译时实际
执行的完整 SQL 字节计算；一个组件可能由多个片段拼接，不能仅对单个文件求哈希。
安装结构只建立数据合同，业务启用仍由配置、权限和实际依赖决定。

| 操作 | 入口与约束 |
| --- | --- |
| 查看合同 | `schema-manifest` 输出编译时合同，不连接数据库；`check-db` 使用显式配置只读检查目标 |
| 安装空库 | `init-db` 使用独立 installer/owner 配置，只面向新的空 Core 库 |
| 创建正式首位管理员 | `create-first-owner` 离线读取私有 stdin JSON，要求显式配置和符合条件的新库 |
| 开发身份引导 | `bootstrap-user` 仅用于开发；一次性凭据写入私有文件 |
| 升级原生库 | 使用与前置合同匹配的离线升级过程、执行器和绑定请求 |
| 普通服务启动 | 使用另一个受限运行账户，校验组件合同、实时 catalog 指纹和权限 |

普通启动不创建角色、不授权、不执行 DDL，也不自动升级已有库。命令语义以
[`main.rs`](../services/core/src/main.rs) 和待运行二进制帮助为准；配置路径见
[文件布局](filesystem-layout.md)，安装顺序见 [Arch Linux 安装和更新](archlinux-install-update.md)。

离线升级绑定指定的 PostgreSQL 前置结构、目标实例、角色、catalog、行与序列，
并保留原请求身份用于恢复。历史升级必须使用其匹配的已保留执行器；新版本二进制
不自动成为所有旧库的通用迁移工具。更新源码或重启服务不等于完成数据库升级。
已有原生库不得通过清空或重新初始化来绕过升级。

## 旧数据迁移与证据

旧单体数据从只读备份恢复的隔离副本读取，写入另一个隔离目标库。
[`migrate-legacy-snapshot.py`](../scripts/migrate-legacy-snapshot.py) 默认预览；身份迁移
只处理账号与受控密码验证信息。旧 API Key、会话、余额、订阅及业务历史使用各自的
受控导入和对账过程，无法确认的权限与登录方式保持关闭。

实际写入须绑定来源、目标及结构凭据，并核对原子事务、原键重放和冲突拒绝。
生产数据、映射、预览及连接信息不得提交仓库。财务工具的 `--source-rights-preview`
只核对冻结导出的经济权利，不生成导入授权，也不证明资金迁移完成；步骤见
[历史余额导入](legacy-financial-opening.md#先定位待处理的经济权利)。

早期 v2/v3 身份、公告、只读 HTTP 及 Web 暂存记录保留在
[固定历史版本](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/core-migration.md)。
这些批次的数量和结果只适用于各自输入。新候选须绑定自己的源码、命令、退出码和
日志，分别记录单元、模拟渠道、真实数据库、真实进程、浏览器及外部供应商证据。

[项目进度](progress.md) · [开发命令](development.md) ·
[隔离安装](../deployment/docker/README.md) · [分发包](release-architecture.md)

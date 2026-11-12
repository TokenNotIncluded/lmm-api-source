# 开发与测试

[工作指南](agent-workflows.md) · [架构](core-migration.md) · [配置路径](filesystem-layout.md)

开发按组件准备工具和隔离环境。以下初始化示例只用于全新隔离库；已有站点使用其
经过审核的升级或迁移流程。

## 仓库与主线

| 仓库 | 主线 | 用途 |
| --- | --- | --- |
| [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) | `main` | Rust Core、Go Extensions、Web 与集成工具 |
| [lmm-api](https://github.com/TokenNotIncluded/lmm-api) | `main` | Recipkg 配方、产品要求与固定源码 gitlink |
| [lmm](https://github.com/TokenNotIncluded/lmm) | 以子项目规则为准 | 客户端，由 `apps/cli` 固定提交 |

继续使用现有工作区和 `main`。先检查版本与本地修改，写入共享分支前检查远端；
保留并行工作，不为每轮任务新建分支或 worktree，不强推、重置或清理他人改动。
跨仓库修改先在子项目自测、提交并推送，再更新父仓库 gitlink；源码提交、制品发布
和生产部署分别记录，授权按实际动作沿用，不能由一次提交推断其他动作已获授权。

旧单体比较固定到 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118)
的 `cc52aa195c6f2a01d703a1a407be73c31b89867d`。旧问题需在当前实现核对和移植，
不能仅改 PR 目标；历史失败、未运行检查和资金义务保留。早期 WIP 交接见
[固定历史记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/development-branches.md)。

[Extore 集成](extore.md)另有部署边界：不得合并或推送会自动部署的 Extore `main`，
不得据此部署。继续前核对其分支、已有修改与两端候选版本。

## 只准备所需工具

版本以 [mise.toml](../mise.toml) 为准；已有匹配工具和锁定依赖可复用。命令使用
`mise exec --`，just 的 recipe 已通过 mise 执行；也可直接调用已准备好的固定版本工具。

| 任务 | 准备内容 |
| --- | --- |
| 文档或技能正文 | Python、Git |
| Rust 核心 | Rust、rustfmt/clippy；数据库测试另需隔离 PostgreSQL |
| Go 扩展 | Go 与受影响模块的测试库，不连接 Core 数据库 |
| Web | Bun/Node.js；首次需要依赖时执行 `mise exec -- just setup` |
| CLI | 初始化 `apps/cli`，使用其固定工具链 |
| Docker 或原生打包 | [Docker 指南](../deployment/docker/README.md)或[原生包指南](../packaging/linux/README.md)列出的工具 |
| pgroll | 仅在对应实验中准备 pgroll 和隔离 Docker |

`mise install` 准备整个工具集，`just setup` 安装 Web 依赖；二者都不是每轮必跑步骤。
只在涉及 CLI 或检查要求完整子模块状态时初始化客户端：

```sh
git submodule update --init --depth 1 -- apps/cli
```

CLI 先在子仓库验证、提交并推送，再更新本仓库指针。质量工具拒绝未初始化、版本
不符或有未提交源码的 CLI；普通文档检查不依赖它。测试 DSN 仅注入所需子进程，
不写入 mise 配置，不传给实际扩展服务。

## 核心与数据库

Core 独立于 Go，身份和账户功能依赖 PostgreSQL。安装与升级合同见
[架构说明](core-migration.md#安装权限与结构升级)和 [schema](../services/core/schema/README.md)。
普通启动只检查结构、catalog 指纹和运行权限，不创建角色、授权、执行 DDL 或升级库。
已有库不得通过清空、重复初始化或修改契约记录绕过检查。

初始化 owner 与受限 runtime 使用分开的凭据，按[离线角色说明](../deployment/postgres/README.md)
准备授权；owner URL 不挂载给运行服务。配置采用 TOML 和受控的绝对路径文件，
不扫描目录、不混入环境覆盖，私密文件须归属服务账户，权限 `0600` 或可读的 `0400`。
[原生路径](filesystem-layout.md)为 `/etc/lmm.conf.d/core.toml`、
`/var/lib/lmm/core` 和 `/run/lmm/core`；模板见 [core.example.toml](../config/core.example.toml)。
`database.url_file` 读取 `/etc/lmm.conf.d/secrets/core/database-url` 中的 runtime URL；
把 owner 凭据交给网络服务会在监听前拒绝。

只对已明确选定的全新隔离空库执行一次安装；运行检查使用另一个受限配置：

```sh
lmm-core --config /absolute/private/core-owner.toml init-db
lmm-core --config /absolute/private/core.toml check-db
```

`check-db` 输出 JSON，只读检查目标结构和配置所需权限，不启动 HTTP、RPC、worker
或模型流量。退出 `0` 表示该次检查通过；备份、物理数据库身份和恢复就绪另行核验。
历史余额导入使用[财务导入合同](legacy-financial-opening.md)，不属于新库安装。

[`scripts/core-ops.py`](../scripts/core-ops.py) 的 `plan`、`status`、`reconcile` 接受显式
安装记录、文件摘要和私有输入，可先运行 `python3 -B scripts/core-ops.py --help`。
`plan` 在私有材料目录独占创建计划；其余两项只读，UNKNOWN 不重放。
物理身份未验证返回 `PHYSICAL_IDENTITY_UNVERIFIED` 和退出码 `3`；工具不授予维护或恢复权限。

本地 HTTP 显式设 `identity.web.local_http=true`；配置 `identity.web.origin` 和
`identity.web.refresh_key_file`，并使 `user_agreement_enabled`、`privacy_policy_enabled`
符合公开协议。注册用 `identity.registration_enabled`；[MFA keyring](core-identity.md#可选-totp-二次验证mfa)
另行配置，未配置也不会绕过已有账号的 MFA。模型、支付默认关闭，健康检查范围见
[独立启动](core-migration.md#独立启动与资源)。

## 启动扩展与前端

先准备扩展的独立完整 TOML 和私有凭据，再从仓库根目录运行：

```sh
(cd services/extensions && mise exec -- go run ./cmd/extensions --config /absolute/private/extensions.toml)
mise exec -- bun run --filter @lmm/web dev --port 5173 --host 127.0.0.1 --strict-port
```

[Extensions 模板](../config/extensions.example.toml)的 `host.token_file` 是独立宿主
凭据；`host.modules=["none"]` 关闭受保护模块。identity 还需 RPC socket、独立口令
和跨用户文件权限。Go 不加载 `.env`，拒绝 `SQL_DSN`、`DATABASE_URL`、
`LMM_CORE_DATABASE_URL` 等旧单体/Core 库变量。两服务的 `--legacy-env` 仅供既有
测试兼容，与 `--config` 互斥，不作为新部署模板。

公开协议页面调试可将 `VITE_REACT_APP_SERVER_URL` 指向本地扩展；仅
[协议指南](legal/README.md)列出的公开 API 不需要 Core。前端构建与业务接通分别
验证；OpenUI 使用明确组件契约，不再向已删除的 Go controller 生成文件。

## 验证与分发

使用[检查选择表](agent-workflows.md#按范围选择检查)，遵守[适度与停止](agent-workflows.md#适度与停止)。
Core-only 工作直接调用核心工具链，不运行同时构建 Core、Go、Web 的 `just build`。
归档与原生包见[独立分发](release-architecture.md)；保留匹配的构建缓存和原始来源记录。

## GitHub Actions

工作流位于源码仓库，自动检查按路径筛选，镜像发布与 S11 源码快照保留手动入口。
实际触发器、输入、权限及 Actions 启用状态以当前仓库和文件为准；修改配置不自动
改变已关闭的 Actions 设置，也不授予触发或部署权限。

| 文件 | 职责 |
| --- | --- |
| [ci.yml](../.github/workflows/ci.yml) | 手动服务端、前端与翻译诊断 |
| [core-protocol.yml](../.github/workflows/core-protocol.yml) | Rust/Go、协议及一次性数据库检查 |
| [microservice-distribution.yml](../.github/workflows/microservice-distribution.yml) | 三个程序与静态 Web 的独立预览归档 |
| [microservice-images.yml](../.github/workflows/microservice-images.yml) | Core/Go 构建、冒烟与 GHCR 预览镜像 |
| [s11-source-snapshot.yml](../.github/workflows/s11-source-snapshot.yml) | 固定源码及 CLI，保存可恢复的 Git bundle 和来源记录 |
| [linux-packages.yml](../.github/workflows/linux-packages.yml) | 原生预览包及隔离安装、升级、运行、卸载 |
| [store-claim-refund-review.yml](../.github/workflows/store-claim-refund-review.yml) | 商店提货/退款的服务端、前端和浏览器回归 |

记录源码、输入、命令、退出码和日志，区分成功、失败、取消与未运行；复用
[本地证据工具](../scripts/local-release-tests.py)或[检查记录校验器](../scripts/verify-release-commit-checks.sh)
时保留其来源约束。归档的所有组件绑定同一固定 SHA；S11 只冻结源码，宿主、资金
及最终组合仍按[进度表](progress.md)独立验收。旧单体工作流见[归档](archive.md)。

预览归档、镜像、正式 Release 和生产部署分别记录。路径检查不覆盖全仓，静态
检查不替代实际运行，不能用空执行消除失败。镜像入口见[Docker 指南](../deployment/docker/README.md#github-container-registry-预览镜像)。

## 按需：pgroll 实验

pgroll 是 mise 固定的外部实验工具，不进入运行镜像、分发包、启动或 `init-db`，
也不作为现有 Core 库升级方案；边界见 [schema](../services/core/schema/README.md#pgroll-evaluation-boundary)。

[实验测试](../tests/pgroll/test_pgroll.py)只连接 `/var/run/docker.sock`，不接收外部 DSN
或读取已有库凭据。它核对新建随机空容器的所有权，只清理自有容器；指定摘要的
镜像须预先准备，测试不下载镜像：

```sh
mise install aqua:xataio/pgroll@0.16.3
mise exec -- just pgroll-version
mise exec -- docker --host unix:///var/run/docker.sock pull postgres:18-alpine@sha256:77f585114c32fbca283dc835b0596f4e52b51b4c6662d7810b2f4084f60a1873
mise exec -- just test-pgroll
```

实验覆盖 init、validate、start、rollback、complete 和两版本视图读写。`validate`
需要真实数据库，`status` 也可能升级旧元数据；不得把它们当只读探针，不得对现有
Core 库运行 init、baseline、convert 或迁移命令。

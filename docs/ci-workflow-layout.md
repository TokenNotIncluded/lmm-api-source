# GitHub Actions 与本地交付入口

[文档目录](README.md) · [新架构进度](progress.md) · [发布架构](release-architecture.md)

工作流文件描述可调用的检查或制品流程。实际触发器、仓库/分支限制、输入与权限以当前文件为准；迁仓后保留的旧绑定整改统一跟踪于 `progress.md` 的 REL-02。流程存在不证明本仓库已启用 Actions 或当前提交已运行。

## 入口与范围

| 文件 | 职责 |
| --- | --- |
| [ci.yml](../.github/workflows/ci.yml) | 手动诊断，调用服务端、前端与翻译检查 |
| [core-protocol.yml](../.github/workflows/core-protocol.yml) | Rust/Go 实现、协议与一次性数据库检查 |
| [microservice-distribution.yml](../.github/workflows/microservice-distribution.yml) | 三个程序与静态 Web 的独立预览归档 |
| [microservice-images.yml](../.github/workflows/microservice-images.yml) | Core/Go 容器构建、冒烟与 GHCR 预览镜像发布 |
| [linux-packages.yml](../.github/workflows/linux-packages.yml) | 原生 Linux 预览包及隔离安装、升级、运行与卸载 |
| [store-claim-refund-review.yml](../.github/workflows/store-claim-refund-review.yml) | 商店提货/退款的服务端、前端和浏览器回归 |

旧单体工作流仅作为[历史来源](archive.md)。路径触发不代表全仓检查；检查不能改成空执行来消除失败。配置格式、仓库条件和实际制品内容必须一致。

## 证据与发布边界

每次记录实际源码、输入、命令、退出码和日志，分开成功、失败、取消与未运行。复用[本地证据工具](../scripts/local-release-tests.py)或[检查记录校验器](../scripts/verify-release-commit-checks.sh)时保留其来源约束，不能套用不同源码的结果。

预览归档、镜像发布、正式 Release 与生产部署分别记录。镜像入口说明见 [Docker 指南](../deployment/docker/README.md#github-container-registry-预览镜像)；启用或触发外部任务按实际授权办理。生产操作需要对应授权，触发请求被接受不等于部署完成。

## 按改动验证

Markdown 只运行文档检查；改检查器时增加其自测。改工作流或交付逻辑时，根据触及的触发器、协议、拓扑与制品选择检查，具体入口见[工作指南](agent-workflows.md#按范围选择检查)。静态检查不替代实际容器、数据库、应用或生产验收。

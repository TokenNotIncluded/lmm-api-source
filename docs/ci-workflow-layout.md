# GitHub Actions 与本地交付入口

[文档目录](README.md) · [新架构进度](progress.md) · [发布架构](release-architecture.md)

工作流使用 `TokenNotIncluded/lmm-api-source` 的 `main`，自动检查按变更路径筛选；镜像发布与源码快照保持手动入口。迁仓时关闭的 Actions 需单独决定是否启用，本次配置修改不更改仓库的 Actions 启用设置。实际触发器、输入与权限以当前文件为准，验收证据见 `progress.md`。

## 入口与范围

| 文件 | 职责 |
| --- | --- |
| [ci.yml](../.github/workflows/ci.yml) | 手动诊断，调用服务端、前端与翻译检查 |
| [core-protocol.yml](../.github/workflows/core-protocol.yml) | Rust/Go 实现、协议与一次性数据库检查 |
| [microservice-distribution.yml](../.github/workflows/microservice-distribution.yml) | 三个程序与静态 Web 的独立预览归档 |
| [microservice-images.yml](../.github/workflows/microservice-images.yml) | Core/Go 容器构建、冒烟与 GHCR 预览镜像发布 |
| [s11-source-snapshot.yml](../.github/workflows/s11-source-snapshot.yml) | 固定调用提交及其 CLI 子模块，保留各自可还原的 Git bundle 与来源记录 |
| [linux-packages.yml](../.github/workflows/linux-packages.yml) | 原生 Linux 预览包及隔离安装、升级、运行与卸载 |
| [store-claim-refund-review.yml](../.github/workflows/store-claim-refund-review.yml) | 商店提货/退款的服务端、前端和浏览器回归 |

旧单体工作流仅作为[历史来源](archive.md)。路径触发不代表全仓检查；检查不能改成空执行来消除失败。配置格式、仓库条件和实际制品内容必须一致。

## 证据与发布边界

每次记录实际源码、输入、命令、退出码和日志，分开成功、失败、取消与未运行。复用[本地证据工具](../scripts/local-release-tests.py)或[检查记录校验器](../scripts/verify-release-commit-checks.sh)时保留其来源约束，不能套用不同源码的结果。

预览归档由合同检查输出固定源码 SHA，所有组件构建使用同一 SHA。S11 快照只冻结源码，真实宿主、资金和最终组合仍按 [进度表](progress.md) 的 CHECK 项独立验收。

预览归档、镜像发布、正式 Release 与生产部署分别记录。镜像入口说明见 [Docker 指南](../deployment/docker/README.md#github-container-registry-预览镜像)；启用或触发外部任务按实际授权办理。生产操作需要对应授权，触发请求被接受不等于部署完成。

## 按改动验证

Markdown 只运行文档检查；改检查器时增加其自测。改工作流或交付逻辑时，根据触及的触发器、协议、拓扑与制品选择检查，具体入口见[工作指南](agent-workflows.md#按范围选择检查)。静态检查不替代实际容器、数据库、应用或生产验收。

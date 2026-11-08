# 开发主线与源码基线

[文档目录](README.md) · [新架构进度](progress.md)

| 仓库 | 主线 | 用途 |
| --- | --- | --- |
| [lmm-api-source](https://github.com/TokenNotIncluded/lmm-api-source) | `main` | Rust Core、Go Extensions、Web 与集成工具 |
| [lmm-api](https://github.com/TokenNotIncluded/lmm-api) | `main` | Recipkg 配方、产品要求与固定源码 gitlink |
| [lmm](https://github.com/TokenNotIncluded/lmm) | 以子项目规则为准 | 客户端；本仓库 `apps/cli` 固定其提交 |

旧单体比较使用固定发布 [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118)，源码提交 `cc52aa195c6f2a01d703a1a407be73c31b89867d`。不依赖可能已移除或改名的历史分支。

继续使用现有工作区与 `main`，先检查当前版本和本地修改，保留其他任务的工作。不得强推、重置或清理他人改动。跨仓库修改先提交子项目，再更新父仓库 gitlink；源码提交、制品发布和生产部署分别记录。

旧问题需在当前实现中核对行为后修复。只改 PR 目标或主线名称不能证明移植完成；历史未通过、未运行和资金义务继续保留。

[Extore 集成](extore.md)仍受明确限制：**不得合并或推送会自动部署的 Extore `main`，不得据此部署。** 继续该任务时先核对其分支、已有修改与两端候选版本。

早期 WIP 交接与 PR 处置记录保存在[固定历史文档](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/development-branches.md)；它不定义当前操作流程。

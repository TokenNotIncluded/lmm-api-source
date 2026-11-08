# LMM API 新旧架构检查：固定基线与已核发现

检查日期：2026-11-08（Asia/Taipei）。本文固定检查源码 `53c945dccb3ef1a105261bc6a91c01dfc6f26191`；报告提交自身不改变被审查的产品源码。

## 基线与方法

| 对象 | 固定输入 | 用途 |
| --- | --- | --- |
| 新架构源码 | [lmm-api-source 53c945dc](https://github.com/TokenNotIncluded/lmm-api-source/tree/53c945dccb3ef1a105261bc6a91c01dfc6f26191) | Rust Core、Go Extensions、Web、测试与交付配置 |
| 配方与验收合同 | [lmm-api 8f11ef82](https://github.com/TokenNotIncluded/lmm-api/tree/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6) | A01–A10、S11、支付对齐和生产切换要求 |
| 历史稳定发布 | [go-v0.2.118](https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118) | 旧单体能力的可定位版本；发布目标为 `cc52aa195c6f2a01d703a1a407be73c31b89867d` |

配方仓库的源码 gitlink 与被审源码一致。检查以实际入口、实现、调用方、测试范围和当前发布证据为依据。文档中的旧日期、旧源码和已通过数量分别保留其原适用范围；明确拒绝、缺配置、源码缺失和未执行验收分别记录。

完整目标沿用 [PRODUCTION_ACCEPTANCE](https://github.com/TokenNotIncluded/lmm-api/blob/8f11ef82ac3a20e33406f2cc7478b419e74bfbe6/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)；总体百分比需要完整、加权并已验收的能力清单，本次不沿用历史“60%/40%”估算。

## 已核的功能进展

- Core 已有 Embeddings/Rerank 入口、Passkey、原生渠道控制、订阅与奖励代码；这些能力不应继续列为“从零缺失”。后续判断需继续区分具体子能力和实际运行验收。
- Core/Go 的旧验收记录已经包含真实进程、隔离 PostgreSQL、资金和慢消费者场景。每份记录仅用于其固定输入和覆盖范围。
- Extore 连接列表、商品同步、只读约束和 UI 已进入源码；真实两端授权、撤销、多规格和重复同步的端到端验收仍待完成。

依据：[模型入口](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/model_http.rs)、[原生渠道](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/core/src/channel_http.rs)、[当前集成记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/integration-verification.md)、[Extore 交接](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/docs/extore-shop-binding-followup.md)。

## 已核的产品缺口

| 入口 | 实际行为 | 需要完成 |
| --- | --- | --- |
| 脚本仓库配置与拉取 | GET/PUT `/api/scripts/repository` 与 POST `/api/scripts/repository/pull` 固定进入 unsupported handler，返回 501 | 恢复旧仓库管理行为，或作出明确的产品范围调整并同步 UI |
| 非 aggregate 用量分享图 | `/api/share/profile/{token}.svg` 的其他布局直接返回 `PROFILE_SHARE_USAGE_UNAVAILABLE` | 将现有 Core 只读用量接入分享布局；补当前用户及公开分享权限验证 |
| Extore 商品同步 | 本地组件与协议已有实现，跨仓库交接明确端到端和正式验收未完 | 用固定两端版本验证授权、同步、撤销与凭据隔离 |

依据：[operations.go](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/webcompat/operations.go)、[profile_share.go](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/services/extensions/internal/modules/community/profile_share.go)。

## 已核的发布与验收阻断

1. `microservice-images.yml` 第 14 行要求仓库为 `TokenNotIncluded/lmm-api` 且使用旧 WIP 分支或预览 tag；当前文件位于 `lmm-api-source`，条件失配。
2. `s11-source-snapshot.yml` 第 11 行同样绑定旧仓库；`core-protocol.yml` 的 push/PR 分支仍为旧 WIP。
3. `microservice-distribution.yml` 第 93 行检查 `site.example.json`，实际 Extensions Dockerfile 第 24 行安装 `site.example.toml`。
4. `.quality/debt.json` 的 `QD-001` 保持 open/critical，期限为 2026-10-24，reviewer pending、evidence 为空；应依据真实资金链及独立复验回执关闭。
5. 当前源码没有对应的正式 Release；母仓库最新正式发布仍为旧 Go `go-v0.2.118`。源码版本与可安装交付物应分别记录。

依据：[镜像工作流](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/microservice-images.yml)、[S11 工作流](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/s11-source-snapshot.yml)、[协议检查](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/core-protocol.yml)、[分发检查](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.github/workflows/microservice-distribution.yml)、[Dockerfile](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/deployment/docker/extensions.Dockerfile)、[质量债务](https://github.com/TokenNotIncluded/lmm-api-source/blob/53c945dccb3ef1a105261bc6a91c01dfc6f26191/.quality/debt.json)。

## 证据判定

代码检查可确认已接线或固定拒绝；已有测试报告保留原版本；当前候选的全站浏览器、外部供应商、迁移对账和联合验收需要各自回执。没有把过期状态文档、组件数量或测试总数当作整体完成率。报告仅包含公开源码和脱敏结论。

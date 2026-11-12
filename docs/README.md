# 文档目录

[项目首页](../README.md) · [贡献指南](agent-workflows.md#贡献与交付)

**当前进度只看[新架构进度](progress.md)。** 本目录提供稳定的设计、配置和验收方法；历史记录集中在[历史索引](archive.md)。源码开发主线是 `TokenNotIncluded/lmm-api-source/main`，配方仓库通过 gitlink 固定源码，见[分支规则](development.md#仓库与主线)。

## 先看这些

| 目的 | 文档 |
| --- | --- |
| 确认剩余事项与关闭条件 | [新架构进度](progress.md) |
| 理解三个程序与资金/权限边界 | [架构指南](core-migration.md) |
| 配置工具链与隔离开发环境 | [开发指南](development.md)、[Docker 开发栈](../deployment/docker/README.md) |
| 选择检查、保存证据与独立验收 | [工作指南](agent-workflows.md)、[质量指南](quality.md) |
| 安装配置、文件所有权与打包 | [文件路径](filesystem-layout.md)、[发布架构](release-architecture.md) |
| 理解完整产品要求 | [产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md) |

## 业务与接口

| 领域 | 稳定指南 |
| --- | --- |
| 身份与跨进程授权 | [核心身份](core-identity.md)、[核心协议](core-protocol.md) |
| 模型计费与用量 | [供应商计费与原生用量](model-billing.md)、[路由计费合同](../services/core/src/routing/README.md) |
| 请求限制与模型目录 | [限速与 Auto](request-limits.md)、[按次计费与原生模型](text-request-and-native-models.md) |
| 支付、金额与历史 | [支付网关](payment-gateways.md)、[金额合同](payment-money-contract.md)、[钱包历史导入](wallet-history.md)、[财务期初导入](legacy-financial-opening.md) |
| 商店与 Extore | [商店模块](../services/extensions/internal/modules/store/README.md)、[Extore 集成](extore.md) |
| 扩展、助手与工具 | [扩展宿主](../services/extensions/README.md)、[助手模块](../services/extensions/internal/modules/assistant/README.md) |
| 滥用防护与管理 | [Abuse controls](abuse-controls.md)、[管理客户端](admin-cli.md)、[Core 运维](core-operations.md) |
| 特定旧合同的离线升级 | [请求运维组件升级](request-operations-upgrade.md)、[商店自购与展示汇率升级](commerce-display-rate-upgrade.md)；执行前核对适用前置版本 |

## 界面、协议与协作

[产品、品牌与界面术语](product.md) · [Web 设计](../apps/web/DESIGN.md) · [Logo](../.github/assets/README.md) · [翻译术语](translation-glossary.md)

[用户/隐私/退款协议](legal/README.md) · [来源与归属](core-migration.md#来源与许可) · [安全报告](../.github/SECURITY.md) · [支持](SUPPORT.md) · [CLI 子项目](https://github.com/TokenNotIncluded/lmm#readme)

[RT16 Markdown 安全复现](../scripts/security/rt16/README.md)只验证其组件范围；[部署研究](deployment-research.md)保留实验结论。二者不替代完整业务或部署验收。

## 维护约定

每个事实只维护一处：进度在 `progress.md`，产品范围在配方要求，接口细节在拥有该接口的模块。删除过时专题时，将固定版本来源加入 `archive.md`，不新增占位页。保留许可证、事故、安全记录和仍在使用的合同；历史通过结论不自动适用于当前源码。

# LMM API

<!-- impeccable:product-schema 1 -->

## Platform

web

## Scope

本记录统一公开入口的产品定位、品牌与界面术语。功能完成状态只维护在[新架构进度](progress.md)，应用布局见 [Web 设计](../apps/web/DESIGN.md)。

## Users

面向模型调用者、MCP 工具创作者、开源贡献者和自建实例的开发者。

## Product Purpose

LMM API（Language Model Mesh，语言模型网络）提供多供应商模型网关、版本化 MCP 工具市场与开源赏金协作。三个程序的职责及资金、权限归属见[架构指南](core-migration.md)。

## Capabilities and Constraints

- 远程 MCP 发布登记已有的公开 HTTPS 服务，不上传或托管任意服务端代码；加载工具、授权调用和授权消费是独立操作。
- 成功付费调用按配置记入创作者收益和平台费用；平台余额可消费模型与工具，不支持提现。平台费率由实例配置，不写成软件固定承诺。
- 源码、模块测试、组件发布和生产部署分别记录；旧站能力不能直接计为新架构完成。

## Brand

产品名统一为 **LMM API**，官方目的地为 lmm.best。[用户原稿](../.github/assets/lmm-api-source.png)是品牌依据：[共享几何](../.github/assets/logo-geometry.json)记录折带符号、几何字标的 SVG 适配与来源。保留比例、黑灰色调、中性阴影和清晰对比。

中英文 README 共用横向 Logo、可访问项目名、标语及原生文本链接；必要说明保持可选择文本。徽章集合由检查器约束，不表达未经核验的发布或上线状态。网站内置标志复用同一几何并继承文字颜色；租户自定义品牌行为保留，旧内置 Logo 路径继续作为缓存客户端别名。

图标使用包含完整宽符号的方形 viewBox；favicon 和 Apple 图标使用浅色方底。16/24/32px、深浅背景及宽窄页头需要视觉检查。修改品牌资源后执行 `mise exec -- python3 -B scripts/check-docs-brand.py`，核对链接、徽章、安全 SVG、路径和摘要；页面布局仍按 Web 设计维护。

## Terminology

同一界面概念使用同一主名称；路由、权限字段和历史数据标识不随显示名称重命名。

| 概念 | 英文主名称 | 中文主名称 |
| --- | --- | --- |
| 模型目录、可用性与价格 | Models and pricing | 模型与价格 |
| 开源贡献奖励 | Open-source bounties | 开源赏金 |
| 控制台帮助助手 | AI assistant | AI 助手 |
| 申请和审核 API 使用资格 | API access | API 访问权限 |

管理员的 `Models` 维护入口与用户的“模型与价格”区分；`Challenges` 保留独立挑战业务含义。L0/L1/L2 是补充等级，不替代能力描述。多语言用词见[翻译术语表](translation-glossary.md)。

登录后主导航为“使用 AI”“开发者”“生态”“账户”“其他服务”；管理功能在独立“管理员”区域，用户来源在其“运营分析”下。短信和临时邮箱激活属于“其他服务”。

保留 general、forge、personal 配置标识，developer 与 services 用于展示分区；console、chat、personal 权限开关仍按路由控制。展示分区不改变授权。`/playground` 是配置助手的兼容重定向；余额、充值、套餐、邀请沿用钱包及个人设置入口，导航需对应实际功能。

## Evidence on Hand

产品依据包括 [README](../README.md)、[金额合同](payment-money-contract.md)、[赏金规则](open-source-bounties.md)、[发布架构](release-architecture.md)与应用配置。旧 LMM Forge 品牌记录保留在[固定历史版本](https://github.com/TokenNotIncluded/lmm-api-source/blob/8e92080aa3e9cb180103e73cb42e78b87621a31e/docs/product.md)。

## Product Principles

入口先说明用途；执行、授权、计费及预览边界清楚；命令、许可证、上游归属和实际文档保持准确。品牌素材与网站标志共用一套身份。

# Extore 商品同步：未完成任务

[文档目录](README.md) · [开发主线与传统分支](development-branches.md)

## 状态与边界

2026-11-07 续作：用户授权推进 Extore 绑定；仍禁止合并或推送 Extore `main`
（会自动部署）。旧缓存工作区已不存在；旧单体候选提交保留为移植源。

- Extore 任务分支 `feat/shop-binding-20261012`，当前
  `a7b9cfd`（含 public marketplace 脱敏与 sync_revision 排除卡密统计）。
- 旧商城绑定候选 `5c63dd5d03cb7561c7b32d84b9f9a81f16594fde`（父仓历史，非微服务
  main）。其 `apps/api-go/internal/extore` 已移植到微服务
  `services/extensions/internal/extore`；authorize/catalog 已挂到 store。
- 微服务仍缺：持久连接列表、按源同步、上游只读编辑限制、绑定 UI 替换旧
  草稿导入页。配置需 `store.extore_key_file` + `store.extore_redirect_uri`。

**禁止部署；尤其禁止合并或推送 Extore `main`。**
不能把旧单体的配置、凭据或资金权限直接接回微服务核心。

## 最终体验与安全条件

商城点击导入 → 自动跳转 Extore → 未登录先登录 → 勾选商品并批准 →
自动导入所有规格 → 返回商城商品列表。

- 超级管理员可选择所有店铺商品；店长只可访问自己的店铺。
- 导入商品显示来源、同步按钮和上次同步时间。
- 重复同步更新原商品和规格，不重复创建。
- Extore 是商品配置的真相源。商城所有商品和规格配置都须能在 Extore
  配置并一一对应；导入商品不得通过商城编辑绕过上游配置。
- 展示卡密状态、数量；同步不自动发行或补货卡密，保留原有发行权限、
  额度和幂等保护。
- 连接撤销、到期和跨店铺隔离须有效。
- 凭据仅由后端加密保存，不回传浏览器、不写日志。

## 已有实现（用户交接说明，非本轮验收结论）

OAuth 长期连接、`marketplace.sync` 独立权限、Extore 商品/规格配置、
随机私密绑定 ID、商城加密连接与稳定商品/规格映射、自动导入和再次同步 UI。

## 剩余工作

1. ~~检查并阻止 Extore `public_product` 暴露 marketplace~~（`a7b9cfd`）。
2. ~~`sync_revision` 排除卡密统计~~（`a7b9cfd`）；商城侧同步仍须不重置审核。
3. 微服务连接令牌刷新 writer fence、事务和并发保护；验证撤销、到期、跨店铺、
   刷新失败。
4. 商品配置修改入口的上游只读限制；空值、清空配置、精确价格映射。
5. 用绑定 UI（自动导入全部规格 + 同步）替换当前草稿导入页；更新测试与文案。
6. 持久连接列表 / 按 `source_id` 同步 / 来源映射表接到 store。
7. 更新协议 schema、接口契约与多语言文案。
8. 本地集成测试（授权、隔离、多规格、重复同步、卡密统计、撤销、凭据不泄露）。
9. 仍不合并 Extore `main`，不部署；记录固定 Extore 任务分支指针。

## 交接时的检查结果（未由本轮重跑）

- Extore：56 通过、1 失败；失败为协议 schema 文件未更新。
- 商城前端类型检查失败，包括旧测试 props 与卡密统计类型。
- Go 定向命令通过，但 `internal/extore` 和 `model` 没有匹配测试，不能证明
  新功能验收通过。用户未提供这些检查的完整命令与精确实测源码快照。

用户提供的历史日志位置（临时路径不保证长期保留，本轮未核验内容）：
`/tmp/extore-binding-oauth-tests-v2.log`、
`/tmp/api-extore-web-typecheck-initial.log`、
`/tmp/api-extore-binding-tests-initial.log`。

后续记录实际命令、实测提交/未提交差异、结果、日志与跳过项；未验收项保持未完成。

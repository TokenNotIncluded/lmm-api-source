# Extore 商品同步：未完成任务

[文档目录](README.md) · [开发主线与传统分支](development-branches.md)

## 状态与边界

2026-11-07 续作：用户授权推进 Extore 绑定；仍禁止合并或推送 Extore `main`
（会自动部署）。

- Extore 任务分支 `feat/shop-binding-20261012`：协议 schema 已补
  `marketplace.sync` / `sync_revision` / `card_summary` 文档对齐（本机已测
  `tests/test_commerce_schema.py` + oauth 相关用例）。
- 微服务源码已挂 authorize、callback 自动同步、连接列表、按源同步、刷新
  fence、上游只读、自动导入 UI、多语言文案与定向单测。
- 配置需 `store.extore_key_file` + `store.extore_redirect_uri`。
- 端到端联调、生产配置与正式验收仍未完成。

**禁止部署；尤其禁止合并或推送 Extore `main`。**

## 最终体验与安全条件

商城点击导入 → 自动跳转 Extore → 未登录先登录 → 勾选商品并批准 →
自动导入所有规格 → 返回商城商品列表。

- 导入商品显示来源、同步按钮和上次同步时间。
- 重复同步更新原商品和规格，不重复创建；同 revision 只刷新卡密统计。
- Extore 是商品配置的真相源；导入商品不得通过商城编辑绕过上游。
- 同步不自动发行或补货卡密。
- 连接撤销、到期和跨店铺隔离须有效；凭据不回浏览器、不写日志。

## 微服务入口

- `POST /api/store/extore/authorize`
- `POST /api/store/extore/catalog`
- `GET /api/store/extore/connections`
- `GET /api/store/extore/connections/{id}/catalog`
- `POST /api/store/extore/connections/{id}/sync`

## 剩余工作

1. ~~协议 schema 文档与 live schema 对齐~~（含 `marketplace.sync`、
   `sync_revision`、`card_summary`）。
2. ~~微服务连接刷新 fence / 上游只读 / 绑定 UI / 映射表 / 定向单测 / 文案~~。
3. 端到端联调：真实 Extore 授权、隔离、多规格、重复同步、撤销、凭据不泄露。
4. 生产配置与部署验收（另批）；仍不合并 Extore `main`。

## 本轮检查

- `go test ./internal/modules/store/ -run Extore`：通过。
- `bun test` Extore import/sync 前端定向用例：通过。
- Extore `tests/test_commerce_schema.py` + oauth 相关：本机通过。
- 未跑生产部署，未声称业务验收完成。

# Extore 商品集成

[文档目录](README.md) · [新架构进度](progress.md) · [商店模块](../services/extensions/internal/modules/store/README.md)

本页定义 Extore 授权与商品同步合同。两端真实验收及生产配置状态统一记录在 `progress.md` 的 CHECK-06。**不得部署，也不得合并或推送会自动部署的 Extore `main`。** 继续跨仓库工作前先核对实际分支、已有修改与两端候选提交。

## 用户流程

商城选择导入，跳转 Extore 登录并批准商品授权，返回后自动同步商品与全部规格。列表显示来源、同步按钮和上次同步时间；连接管理支持查看与按源同步。

Extore 是导入商品配置的权威来源。商城不能绕过上游修改受控字段；同一来源重复同步更新原商品和规格，同 revision 只刷新卡密统计。同步不自动发行或补货卡密。

## 配置与接口

Go 商店配置需要 `store.extore_key_file` 和 `store.extore_redirect_uri`。使用实际服务账户可读取的私密凭据文件，回调地址与授权端注册值一致。

| 方法与路径 | 用途 |
| --- | --- |
| `POST /api/store/extore/authorize` | 创建授权跳转 |
| `POST /api/store/extore/catalog` | 授权后的目录读取 |
| `GET /api/store/extore/connections` | 当前店铺连接 |
| `GET /api/store/extore/connections/{id}/catalog` | 指定连接的目录 |
| `POST /api/store/extore/connections/{id}/sync` | 按连接同步 |

实现入口：[Extore handler](../services/extensions/internal/modules/store/extore.go)、[商店说明](../services/extensions/internal/modules/store/README.md)。回调、刷新 fence 和上游只读检查属于同一授权链，不能由浏览器声明覆盖服务端状态。

## 验收合同

在同一冻结候选上验证授权、回调自动导入、多规格、重复同步、同 revision 更新、撤销、到期、刷新竞争和跨店铺隔离。核对实际数据库映射及 Web 请求；凭据不得返回浏览器或进入日志。

模块与前端测试只证明其各自范围。真实两端联调、生产配置、部署和上线签收分别留证。2026-11-07 的分支、执行命令与原始限制保存在[固定交接记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/extore-shop-binding-followup.md)。

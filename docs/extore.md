# Extore 商品集成

[文档目录](README.md) · [新架构进度](progress.md) · [商店模块](../services/extensions/internal/modules/store/README.md)

本页定义 Extore 授权、目录传输与商品同步合同。两端真实验收及生产配置状态统一记录在 `progress.md` 的 CHECK-06。Extore 的 `main` 会自动部署；继续跨仓库工作前核对实际分支和固定提交，插件依赖只引用已审查的固定版本。

## 用户流程

商城选择导入，跳转 Extore 登录并批准商品授权，返回后自动同步商品与全部规格。列表显示来源、同步按钮和上次同步时间；连接管理支持查看、按源同步与断开连接。

断开连接需要当前店铺的管理权限。服务端先向 Extore 撤销该连接的上游授权，成功后才在本地停用连接；上游不可达或撤销失败时保留本地连接，以便重试。断开不会删除已导入商品或来源历史；该连接不再列为可用，后续目录读取和同步需要重新授权。

Extore 是导入商品配置的权威来源。商城不能绕过上游修改受控字段；同一来源重复同步更新原商品和规格，同 revision 只刷新卡密统计。同步不自动发行或补货卡密。

## 配置与接口

Go 商店配置需要 `store.extore_key_file` 和 `store.extore_redirect_uri`，以及原有的 Store PostgreSQL 与 Core 权限配置。密封密钥是独立的原始 32 字节私密文件，不能复用库存密钥；保持既有密钥才能继续解密已保存连接。文件由实际服务账户读取，权限遵守[私密文件合同](filesystem-layout.md#权限与服务账户)。

回调地址必须是规范的 `https://<站点域名>/store/manage`，没有查询或片段，并与浏览器当前 origin、`host.public_origin` 和授权端注册值一致。现有 Web 入口默认使用 `https://extore.lmm.best` 和公开客户端 `lmm-best-store`；PKCE 流程不要求客户端秘密。Extore 的既有内置客户端注册允许 `https://api.lmm.best/store/manage` 和 `https://native.api.lmm.best/store/manage`。其他站点须先完成相应客户端注册，不能只修改回调字符串。

| 方法与路径 | 请求与用途 |
| --- | --- |
| `POST /api/store/extore/authorize` | `{base_url, client_id}`；创建授权跳转，空值使用默认服务和客户端 |
| `POST /api/store/extore/catalog` | `{callback_url}`；消费一次回调、保存连接并同步批准的商品 |
| `GET /api/store/extore/connections` | 当前用户有权管理的连接及商品来源 |
| `GET /api/store/extore/connections/{id}/catalog` | 指定连接的授权目录 |
| `POST /api/store/extore/connections/{id}/sync` | `{source_id}`；同步一个批准的商品 |
| `POST /api/store/extore/connections/{id}/sync-all` | 按连接同步批准的目录 |
| `POST /api/store/extore/connections/{id}/disconnect` | 撤销上游授权并停用本地连接，保留来源历史 |

实现入口：[授权与连接](../services/extensions/internal/modules/store/extore.go)、[目录与同步](../services/extensions/internal/modules/store/extore_sync.go)。连接记录的 `shop_id` 来自上游 Extore 目录，不是 LMM 本地店铺 ID。读取目录时同时核对 issuer、grant、商品归属及已保存的上游店铺 ID；浏览器声明不能覆盖这些服务器状态。凭据以连接 ID 绑定的密文保存，不返回浏览器。刷新和撤销继续使用既有持久化 fence。

## 可选原生目录插件

未配置 `store.extore_plugin` 时，目录继续使用 Go HTTPS 客户端。配置后，Go 在一个专用操作系统线程上通过 purego 调用 Rust 宿主桥；宿主使用 zplugin 的实际加载器和 `extore.commerce` v1 类型接口。Rust 插件通过 HTTPS 读取真实 Extore 授权目录，Go 再执行相同的目录和 Store 权限校验。发现服务、OAuth、刷新和撤销仍使用现有 Go HTTPS 路径。

在已经启用 Extore 授权的 `extensions.toml` 中增加：

```toml
[store.extore_plugin]
host_library = "/usr/lib/lmm/extensions/extore/libextore_plugin_host.so"
plugin_library = "/usr/lib/lmm/extensions/extore/libextore_commerce_plugin.so"
host_sha256 = "<该平台宿主库的 64 位小写 SHA-256>"
plugin_sha256 = "<该平台插件库的 64 位小写 SHA-256>"
# 可选：额外信任的 CA PEM，证书与主机名验证仍保持开启
# ca_file = "/etc/lmm.conf.d/extore-ca.pem"
```

两个摘要取自同一可信发行归档的 `MANIFEST.json`；原生包安装的 `/usr/share/doc/lmm-extensions/BUILDINFO.json` 也记录安装路径及摘要。库必须是有界的普通文件，不允许组或其他用户写入。程序先校验摘要，再通过保留的文件描述符加载对应文件。GNU 与 musl 库各有自己的摘要，不能互用。

开启插件后，路径、摘要、ABI、CA 或初始化错误会阻止启动；运行中的插件调用错误返回到调用者，不会静默改用 Go 目录传输。每次请求的创建、调用、输出释放和实例关闭均在同一个固定线程上完成；调用者取消等待后，工作线程仍完成资源清理。排队和等待结果共享 20 秒预算，原生 HTTPS 另有 5 秒连接和 20 秒请求上限，目录上限为 16 MiB，额外 CA 上限为 256 KiB。允许的上游是规范 HTTPS DNS origin 和标准 443 端口，不接受 IP、用户名密码、非根路径、查询或片段；不跟随重定向。

可在实际服务账户、组、工作目录和加载器环境中运行只加载检查：

```sh
/usr/bin/lmm-extensions --check-extore-plugin --config /etc/lmm.conf.d/extensions.toml
```

该模式严格解析同一 TOML，只校验并加载两个原生库、创建和关闭一个实例，然后退出；不读取业务凭据、不初始化运行时、不连接数据库，也不请求上游。成功返回退出码 0 和明确的加载结果。它只证明当前进程环境能加载已配置插件，不代表 OAuth、数据库或用户授权已经可用。

服务关闭会等待当前原生调用结束并退出专用线程。两个原生映像及其校验后的文件描述符保留到进程退出；更新库路径、内容摘要或包版本后必须重启进程，不能把 `Close` 描述为卸载共享库。原生目录传输不会新增产品程序或守护进程。

## 发行文件

Extore 源码以 `packages/extore` 的固定 gitlink 引入，版本记录在 [extore-plugin.json](../packaging/extore-plugin.json)；构建前运行 `scripts/check-extore-plugin-pin.py` 核对。正式 Extensions 包包含两份独立共享库、Extore 许可证、锁定依赖的许可证原文与清单，以及 purego 和 Go 衍生代码的许可证。

| 归档平台 | 原生包 | 动态库与解释器 |
| --- | --- | --- |
| `linux-amd64`、`linux-arm64` | DEB、RPM、Arch | GNU 构建的两份共享库，检查实际 glibc 最低版本 |
| `linux-musl-amd64`、`linux-musl-arm64` | APK | musl 构建的两份共享库及对应 musl 解释器，声明 musl 和 libgcc 依赖 |

Go 两种产物均保持 `CGO_ENABLED=0`。purego 需要动态加载器，因此该设置不表示程序静态链接。APK 使用单独构建和标记的 musl 归档，完整安装矩阵及验证命令见 [Linux 预览包](../packaging/linux/README.md)。

## 验收合同

在同一冻结候选上验证授权、回调自动导入、多规格、重复同步、同 revision 更新、撤销、到期、刷新竞争和跨店铺隔离。核对实际数据库映射及 Web 请求；凭据不得返回浏览器或进入日志。

原生开发消费者使用真实 Rust Extore、HTTPS、SQLite 和共享库，并检查默认 Go HTTPS 路径、错误摘要、路径与 ABI、授权范围、Store 连接归属和关闭顺序。该消费者的 Store 仓库为事务内存测试仓库，Core 权限为现有测试替身；不能代替真实 CoreSession、Store PostgreSQL 和浏览器的独立验收。Alpine 包检查还必须实际安装 APK、以服务账户启动未配置插件的已安装程序，并用已安装的两份共享库调用真实授权目录。

模块测试、两端联调、安装验收、生产配置和部署分别留证。2026-11-07 的分支、执行命令与原始限制保存在[固定交接记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/extore-shop-binding-followup.md)。

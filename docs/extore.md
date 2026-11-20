# Extore 商品集成

[文档目录](README.md) · [新架构进度](progress.md) · [商店模块](../services/extensions/internal/modules/store/README.md)

本页定义 Extore 授权与商品同步合同。两端真实验收及生产配置状态统一记录在 `progress.md` 的 CHECK-06。**不得部署，也不得合并或推送会自动部署的 Extore `main`。** 继续跨仓库工作前先核对实际分支、已有修改与两端候选提交。

## 用户流程

商城选择导入，跳转 Extore 登录并批准商品授权，返回后自动同步商品与全部规格。列表显示来源、同步按钮和上次同步时间；连接管理支持查看、按源同步与断开连接。

断开连接需要当前店铺的管理权限。服务端先向 Extore 撤销该连接的上游授权，成功后才在本地停用连接；上游不可达或撤销失败时保留本地连接，以便重试。断开不会删除已导入商品或来源历史；该连接不再列为可用，后续目录读取和同步需要重新授权。

Extore 是导入商品配置的权威来源。商城不能绕过上游修改受控字段；同一来源重复同步更新原商品和规格，同 revision 只刷新卡密统计。同步不自动发行或补货卡密。

## 配置与接口

Go 商店配置需要 `store.extore_key_file` 和 `store.extore_redirect_uri`。使用实际服务账户可读取的私密凭据文件，回调地址与授权端注册值一致。

授权开始与回调都由 Core 验证当前 Session。Core 的 `AuthorizeResponse.session_binding` 是带版本的、不含凭据的摘要，由同一只读认证快照中的用户、权限代际和浏览器会话族生成；Go 只接受 Core 返回的绑定，不解析 access token，也不读取 Core 数据库。同一浏览器会话正常刷新 access token 后仍可完成原授权；同一用户另一次登录、权限代际变化、撤销和失效不能接用原流程。原生 Session 没有浏览器会话族时按已验证的凭据绑定，不能退化成只按用户绑定。

此合同需要配套的 Core 和 Go Extensions。旧 Core 未返回绑定时，Go 对 Extore 授权返回服务不可用（503）；其他使用原 `Authority.Check` 的商店操作不受这个新增字段影响。已开始的流程仍受一次性消费、十分钟有效期、加密凭据和版本冲突检查约束；升级前按旧 access token 创建的未完成流程需要重新授权。

当前网页调用授权 API 时传入空的 `base_url` 和 `client_id`，因此使用 `https://extore.lmm.best` 与 `lmm-best-store`。Extore 的这个内置客户端只接受 `https://api.lmm.best/store/manage` 和 `https://native.api.lmm.best/store/manage` 两个回调地址。其他来源或回调需要在 Extore 注册客户端，并通过已有授权 API 显式传入对应的 `base_url`、`client_id`；更改 `store.extore_plugin.origin` 只改变目录路由，不会改变网页的授权目的地。

| 方法与路径 | 用途 |
| --- | --- |
| `POST /api/store/extore/authorize` | 创建授权跳转 |
| `POST /api/store/extore/catalog` | 授权后的目录读取 |
| `GET /api/store/extore/connections` | 当前店铺连接 |
| `GET /api/store/extore/connections/{id}/catalog` | 指定连接的目录 |
| `POST /api/store/extore/connections/{id}/sync` | 按连接同步 |
| `POST /api/store/extore/connections/{id}/disconnect` | 撤销上游授权并停用本地连接，保留已导入商品及来源历史 |

实现入口：[Extore handler](../services/extensions/internal/modules/store/extore.go)、[商店说明](../services/extensions/internal/modules/store/README.md)。回调、刷新 fence 和上游只读检查属于同一授权链，不能由浏览器声明覆盖服务端状态。

## 原生插件目录（可选）

同一节点可把一个已配置 Extore 来源的目录读取交给原生插件。Go Extensions 启动私有 Rust 宿主，宿主通过 Project Z 的 `zplugin = 0.3.1` 加载真实 Extore 动态库。浏览器继续使用上面的商店接口；只有精确匹配服务端 `origin` 的 `GET /api/integrations/commerce/products` 走私有 Unix socket。授权发现、PKCE、刷新和撤销继续使用原 Extore HTTP 接口，其他来源继续使用 HTTP。已配置插件的目录请求失败时直接报错，不自动改走其他来源。

插件复用 Extore 自己的目录处理器、当前授权记录和商品 revision。每次读取都检查现有授权，因此加载后撤销仍然生效。Go 保留原来的 Core 管理权限、session 绑定、grant/shop 校验、加密凭据和同步 fence；原生插件没有 Core 数据库或资金接口。

在已有商店与 Extore 授权配置下，按[配置示例](../config/extensions.example.toml)增加 `[store.extore_plugin]`。六个字段分别指定来源 `origin`、宿主可执行文件 `host`、动态库 `library`、该库的 `sha256`、私有 `socket` 和既有 Extore `data_dir`。这些值来自服务端配置，浏览器不能指定。数据目录与 socket 父目录须由服务账户拥有且为 0700；宿主创建 0600 socket。代码文件及祖先目录须由 root 或服务账户控制，拒绝 symlink 和可被其他账户替换的路径。已有目录不得由 LMM 启动流程初始化或迁移。

此处的服务账户是 Go Extensions 的实际 UID，私有宿主继承该身份。这个可选模式要求共同部署的 Extore HTTP/worker 与宿主访问同一份实时 Extore 状态，并事先按共同的受信服务身份配置原始数据目录。远端 Extore 或使用不同 UID 的现有服务，需要先完成明确的部署设计；目录副本不能保持实时撤销语义。启用过程不是修改已有目录所有权、扩大权限或搬迁业务数据库的授权。

未配置 `store.extore_plugin` 时保持原 HTTP 接入，也不启动私有宿主；配套 Core/Go 的稳定 Session 修复同样适用于此默认模式。当前固定共享状态 API 与其他分支同名的 HTTPS 代理 API 不兼容，宿主、共享 API、动态库、数据合同和摘要须来自本节同一固定依赖。

宿主目前支持 Linux GNU 原生库，动态库与宿主须使用匹配的目标平台。原生代码属于受信代码，进程分离不等于 OS 沙箱。Go 只向子进程传入上述参数与最小环境；Rust 宿主在同一 OS 线程上配置、调用和释放插件返回的内存，以 `Pinned` 策略保持库映射。目录请求串行执行，响应上限为 16 MiB，原成功与拒绝响应均保留 `Cache-Control: no-store`。

### 构建与源码绑定

`third_party/extore` 固定到 Extore 提交 `8e3063690cf5595884b1e639535869fd593e0296`。完整 Rust 实现来自 `818400958a8660e5e72b1eefb4815dbb34452098`；`1c29a9a0` 修正两个 Python 夹具的店铺 UUID 与网页合同断言，当前提交同步了这两项输入摘要。71 项输入均与实际文件匹配，69 项其他条目以及 Rust runtime、共享 API、静态网页输入未变；历史构建和检查回执仍对应原来源。[宿主 Cargo.toml](../services/extensions/extore-host/Cargo.toml)通过 `version = "=0.1.0"` 和该子模块内的路径依赖使用共享 API，[Cargo.lock](../services/extensions/extore-host/Cargo.lock)锁定其余依赖。接口与实际动态库由同一个 Gitlink 绑定源码，保持 Project Z 对已审阅来源的校验规则。动态库的来源提交、工具链、目标平台和构建产物 SHA256 应一起保存，运行配置中的 `sha256` 使用实际动态库文件的值。

以下命令在 LMM 源码根目录执行，初始化仓库已记录的固定子模块，并构建匹配的 Linux GNU 产物；它们不修改 Extore `main`，不安装或启动服务。已有子模块工作区应先保留其改动，不使用强制更新：

```bash
git submodule update --init third_party/extore
git submodule status third_party/extore
mise exec -- cargo build --locked --release --target x86_64-unknown-linux-gnu --manifest-path third_party/extore/Cargo.toml -p extore-zplugin
mise exec -- cargo build --locked --release --target x86_64-unknown-linux-gnu --manifest-path services/extensions/extore-host/Cargo.toml
sha256sum third_party/extore/target/x86_64-unknown-linux-gnu/release/libextore_zplugin.so
```

宿主产物是 `services/extensions/extore-host/target/x86_64-unknown-linux-gnu/release/lmm-extore-host`；动态库路径如最后一条命令所示。安装时按上述权限提供受控文件与运行目录，再填写实际路径和摘要。这个可选宿主由 Go 管理生命周期，Core 继续独立运行。

### 可选交付包

三个可选交付入口用 `extore_plugin` 布尔输入选择成对交付，默认值为 `false`。选择后，`microservice-distribution.yml` 在选中的 Extensions archive 中加入宿主与动态库；`linux-packages.yml` 在原双架构检查之外生成并检查 GNU 变体；`microservice-images.yml` 只构建和发布 Extensions 的 `-extore` 镜像变体，并保留原有 main 来源准入。默认入口继续生成原 Go-only 产物，配置示例仍不启用插件。

自动 archive 部署入口 `deploy.yml` 保持 Go-only，不提供插件选项。现有部署以 symlink 切换 a/b 槽位，配置文件保留在槽位外；插件的受控路径检查拒绝 symlink，外部配置也不会随槽位切换改写。因此自动部署成功只证明原 Go 服务通过检查，不能保证宿主与动态库已更新、来自同一源码或已实际启用。归档中的成对代码须另行按本节合同安装与配置；原生包及镜像也只交付代码，启用仍须核对实际路径、摘要、共同 UID 和同一份实时 Extore 数据目录。

| 交付形式 | 宿主路径 | 动态库路径 |
| --- | --- | --- |
| GNU archive 内部 | `libexec/lmm-extore-host` | `lib/extore/libextore_zplugin.so` |
| DEB、RPM、Arch Linux 包及可选镜像 | `/usr/libexec/lmm-extore-host` | `/usr/lib/lmm/libextore_zplugin.so` |

Archive 文件名为 `lmm-extensions-<version>-linux-<arch>-extore.tar.gz`；原生包附件名为 `lmm-extensions-extore-<version>-<release>-<arch>.<format>`，包管理器中的包名仍是 `lmm-extensions`。同一次安装选择一种变体，不能把两份同组件包一起交给安装器。镜像使用 `sha-<LMM_SHA>-extore` 和 `preview-<short_SHA>-extore` 标签，与 Go-only 标签分开；发布步骤核对同一个已测试 image ID，拒绝覆盖已有标签。

[归档构建器](../scripts/build-distribution.py)的 `--extore-revision <完整 gitlink SHA>` 是显式开关：输入目录须同时含 `lmm-extore-host`、`libextore_zplugin.so` 和 `extore-plugin.json`。构建记录由 `record-extore` 生成，绑定 LMM 提交、Extore gitlink、精确 GNU target、两个文件的 SHA256、大小和模式；归档再次核对实际字节并把记录写入 `MANIFEST.json.extore_plugin`。缺少一项、架构不符或记录不匹配都会拒绝。记录是可核对的构建元数据，不是签名或实际加载验收。

[Dockerfile](../deployment/docker/extensions.Dockerfile)的 `binaries-extore` 与 `runtime-extore` 目标使用同一固定子模块、锁文件和 Bookworm Rust 工具链；默认 `binaries`、`runtime` 及无 target 构建保持 Go-only。调用者须传入已核对的 `LMM_SOURCE_REVISION` 与 `EXTORE_SOURCE_REVISION`。镜像构建先确认编译器实际 host 与目标架构一致，再在最终 runtime 内解析两份 ELF 的 libc、OpenSSL 等运行依赖；结果写入 `/usr/share/lmm/extore-runtime-dependencies.txt`。仅复制 ELF 或检查文件头不代表可运行。

| Archive 平台 | 精确 Rust target | 原生包边界 |
| --- | --- | --- |
| `linux-amd64` | `x86_64-unknown-linux-gnu` | DEB、RPM、Arch Linux |
| `linux-arm64` | `aarch64-unknown-linux-gnu` | DEB、RPM、Arch Linux |

[原生包构建器](../packaging/linux/build.py)从实际 ELF 读取 GLIBC 和 OpenSSL 最低符号版本并声明依赖，不能只凭 CPU 架构认定发行版兼容。APK 拒绝携带这对 GNU 产物；原 static/Go-only APK 流程继续保留。可选 GNU 容器检查覆盖 Debian、Ubuntu、Fedora，以及 amd64 的 Arch Linux、openSUSE；Rocky Linux 9 保留原 static/Go-only 检查。每个目标仍须取得实际安装结果，不能把静态合同检查当作发行资格。

安装包只交付代码、原配置示例和 Extore 许可证，不创建 Extore 数据库、改变已有目录权限或启动服务。启用时把配置中的 `host`、`library` 指向实际受控安装路径，`sha256` 填入同一包内动态库的真实摘要；`data_dir` 仍须是 Extore HTTP/worker 使用的同一份既有实时状态。Go Extensions 与这些 Extore 进程使用同一受信实际 UID，数据目录和 socket 父目录为 0700，socket 为 0600。

可选 `microservice-distribution.yml` 流程对归档解出的宿主与动态库运行下节 `native-flow.py`。原 Linux 包流程对实际 DEB 解出的同一对文件运行该联调，并在原容器安装、升级、移除流程中核对文件摘要、权限和运行库解析。可选交付的发布记录应保留同一源码、gitlink、manifest、包摘要和这些实际结果；它们不替代 Web/Core 验收、生产迁移审批或流量切换签收。

### 定向自测

[真实联调脚本](../services/extensions/extore-host/tests/native-flow.py)复用 Extore 自己的隔离夹具，以真实 HTTP 注册、PKCE 同意与 token 接口获得两个店铺授权，再通过 Go 客户端、Unix socket 和实际动态库读取目录。它检查连续读取、无授权与跨 grant 拒绝、配置后经原 HTTP 接口撤销、另一店铺仍可读取、私密字段过滤、价格文本保留，以及启动拒绝和 socket 清理。测试凭据只留在临时私密目录中，结束时清理；回执记录源码与产物摘要，不保存凭据。

该自测需要真实 `extore`、`libextore_zplugin.so` 和 `lmm-extore-host` 构建产物，单元测试中的模拟目录不能替代它。Web、真实 Core 会话、部署配置和生产签收仍按下方验收合同分别验证。

## 验收合同

在同一冻结候选上验证授权、回调自动导入、多规格、重复同步、同 revision 更新、撤销、到期、刷新竞争和跨店铺隔离。核对实际数据库映射及 Web 请求；凭据不得返回浏览器或进入日志。

模块与前端测试只证明其各自范围。真实两端联调、生产配置、部署和上线签收分别留证。2026-11-07 的分支、执行命令与原始限制保存在[固定交接记录](https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/extore-shop-binding-followup.md)。

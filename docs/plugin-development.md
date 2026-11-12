# 扩展插件开发与管理

扩展插件属于 `lmm-extensions` 宿主。插件代码是经过审查后编译进宿主的 Go 模块，不能从浏览器或管理员请求上传 `.so`、脚本或任意二进制。这样保留了 Rust Core 的身份、权限和资金边界，也避免把不受信任代码放进同一进程。

## 插件契约

插件实现 `services/extensions/internal/modules.Module`：

```go
type Module interface {
    Name() string
    Handler() http.Handler
}
```

`Name()` 只能使用小写字母、数字和短横线，长度最多 32；服务令牌只负责进入宿主，插件仍须用 Core RPC 核验用户会话和权限。模块不能拿 Core 数据库连接、全局 ORM 或共享余额写入器。每个模块有独立的路由前缀和并发上限。

内置插件在 [`composition/registry.go`](../services/extensions/internal/composition/registry.go) 声明名称、版本、依赖、导航入口和绑定函数。模块若另提供 `Version() string`，必须与声明版本一致；没有提供时采用声明版本。注册摘要是：

```text
sha256("lmm-extension-registration-v1\0" + name + "\0" + version)
```

它绑定管理员请求和当前编译版本，不能替代发布包签名或代码审查。新插件必须在源码中注册、通过相应测试和构建验收后，随新的 `lmm-extensions` 包发布。

## 注册与装配

同一份注册目录同时驱动 `host.modules` 选择、运行模块绑定和管理 API 的目录。新增包不再分别修改启动白名单、末尾 `switch` 和前端路径表。模块的独立存储、凭据与 Core 授权仍由 `composition` 显式装配。

`modules.Descriptor` 中的 `Requires` 声明依赖，例如 `support` 依赖 `store`。配置必须显式选中依赖，不会因安装或注册一个包而自动启用其他包。管理接口也按该声明检查启停：先启用依赖，再启用插件；停用时顺序相反。

`NavigationPaths` 声明该插件拥有的 Web 页面路径。管理 API 返回 `navigation_paths` 和 `requires`，前端据此限制既有菜单，无须再次维护模块名称列表。一个页面可同时受多个包约束；Core 与扩展混合的页面不声明整个前缀。目录只是功能声明，不替代用户权限或服务健康检查。

目录包含当前编译的全部包。未在本次进程中装配的包返回 `available=false`，前端显示“需要配置”；它们不能通过安装或启用 API 获得缺失的数据库和凭据。首次没有状态文件时，仅自动安装和启用已装配模块；已有状态文件中没有的新包保持未安装、未启用。

设置了 `host.plugin_state_file` 时，从 `host.modules` 移除包之前须先停用其依赖方及该包。若持久状态仍要求启用未装配包，启动明确失败；不会丢弃旧状态或静默扩大配置。

## 管理 API

这些接口要求一次性的 Core session Bearer 凭据，并且 `platform_level` 为 5 或 6；宿主服务令牌和用户自带的身份头都不具备管理员权限。

| 方法 | 路径 | 作用 |
| --- | --- | --- |
| `GET` | `/api/extensions/plugins` | 查看当前编译注册、安装和启用状态 |
| `POST` | `/api/extensions/plugins/install` | 安装一个与当前注册摘要完全匹配的 manifest |
| `POST` | `/api/extensions/plugins/{name}/enable` | 启用插件路由 |
| `POST` | `/api/extensions/plugins/{name}/disable` | 禁止新的插件请求 |
| `DELETE` | `/api/extensions/plugins/{name}` | 删除安装状态；必须先禁用 |
| `POST` | `/api/extensions/plugins/reload` | 重新应用当前启用快照 |

安装请求只含 `name`、`version`、`api_version` 和 `registration_sha256`。启停和重载是热操作：路由表用原子快照切换，已进入宿主 admission 的请求不会被强行取消；关闭数据库或 worker 仍由正常进程排空流程负责。

Web 路由在注册时绑定所属插件，宿主自动添加并发准入与启用状态检查。`RouteRegistrar` 与 `http.ServeMux` 使用相同的 `Handle`/`HandleFunc` 方法；模块内部新增接口会继承其注册器的约束，不再补写 URL 前缀判断。部署管理、RSS、脚本及工具市场内置工具也沿实际归属受控。市场审核设置由任一已启用市场提供，商品和工具的审核历史分别受各自插件控制。

插件管理路由独立于业务插件的开关，仍要求管理员身份；健康和原生宿主入口保留自身处理。缺少装配返回 `PLUGIN_NOT_CONFIGURED`（503），依赖启停顺序错误返回 `PLUGIN_DEPENDENCY_REQUIRED`（409）。已接纳请求继续按正常排空流程完成。

## 开发、测试和发布

1. 在 `services/extensions/internal/modules/<name>` 实现模块及窄依赖，在 `composition` 中显式装配，并在 `moduleRegistrations` 添加一个描述及绑定函数；不要在插件中自行打开 Core 数据库。
2. 在 Web 装配入口把模块的注册函数接到 `routes.forPlugins(name)`。模块通过传入的 `modules.RouteRegistrar` 注册实际路由；依赖和菜单入口只在包描述中声明。
3. 为受影响的选择、鉴权、启停与排空行为运行针对性测试；共享宿主或注册接口变化运行 Go 检查：

   ```sh
   (cd services/extensions && go test -race ./... -count=1)
   ```

4. 按新增依赖更新配置示例和分发清单；重新构建并核对源码提交、包摘要和运行时 `BUILDINFO`。
5. 先在带有专用 Core RPC 和隔离数据库的候选宿主启用，验证 `/extensions/v1/modules`、Web 兼容路由、禁用后的 503 和重载期间的请求排空，再考虑生产灰度。测试因权限、数据库或供应商缺失而失败时保留失败，不能当作候选验收通过。

生产安装只替换经过校验的宿主包。`host.plugin_state_file` 是宿主用户可写的私有状态文件，目录应为 0700、文件为 0600；其中没有凭据、session、余额或插件代码。状态损坏或摘要不匹配时宿主拒绝启动，避免静默启用错误版本。

该设计支持管理员启停、卸载和热重载已发布插件；它不把网络请求变成代码发布渠道。真正的第三方插件若需要不受信任执行，必须先设计独立进程或容器协议并单独完成安全和资金验收，不能直接改成进程内动态加载。

# 原生地域与 IP 访问控制

[文档目录](README.md) · [控制台设置](core-console-settings.md)

Rust Core 在公开 HTTP 入口执行平台地域规则，并在身份验证、会话签发、会话刷新和模型调用中执行用户 IP 白名单。个人规则绑定用户，不依赖开发者资格或旧信任等级。Go 扩展和前端不拥有放行权限。

## 配置入口与生效关系

| 位置 | 配置 | 生效范围 |
| --- | --- | --- |
| 系统设置 → 安全设置 → IP 访问规则 | 允许或拒绝所选地域；高级 IP/CIDR 规则；未知地域处理 | Core 的公开业务请求 |
| 个人资料 → 安全 → IP 白名单 | 用户自己维护的 IPv4、IPv6 和 CIDR | 该用户的登录、会话、刷新及 API 密钥请求 |
| API 密钥设置 | 既有单 Key IP 限制 | 该 Key 的模型与只读 Key API |

请求必须同时满足平台规则、用户白名单和该 Key 的限制。用户白名单不会覆盖平台拒绝，也不会放宽 Key 本身的范围。用户白名单为空时不增加用户级限制；平台未配置规则时不增加平台级限制。

平台的 `/health/live`、`/health/ready`、`/health/model-ready` 保留可用性检查语义。私有 UDS 服务 RPC 保持既有服务身份边界。规则更新对后续请求生效，已经开始的流式响应不由这一功能中断。

## 平台地域规则

地域配置器把“仅允许”或“拒绝”选择转换为已有 `IPAccessRoutingRules` 配置。服务端保存、预览与请求检查复用同一解析器。

例如仅允许日本和美国，未知地域拒绝：

```text
ip(geoip:JP, geoip:US) -> direct
fallback: reject
unknown-country: reject
```

例如拒绝指定地域，同时允许其他已识别地域：

```text
ip(geoip:CN) -> reject
fallback: direct
unknown-country: reject
```

这里的地区码只用于说明语法；新增功能不会自动写入这些示例规则。关闭平台限制可保存 `fallback: direct`。

原生解析器支持 `ip(...)` / `dip(...)`、单 IP、CIDR、IP 范围、`geoip:XX`、`geoip:unknown`、`ipversion(4|6)`、前置 `!`、`fallback` 与 `unknown-country`。规则按顺序处理，首个匹配决定结果。多个匹配项用逗号表示“任一满足”。域名、端口和传输协议表达式不属于该解析器支持范围。

遇到需要地域信息而来源地域未知时，使用显式 `unknown-country` 决策；没有配置此项会返回策略不可用，保持拒绝。直接命中更早的 IP 规则不要求存在地域信息。配置界面的预览与实际请求使用同样的规则。

## 本地地域数据库与可信代理

Core 支持在进程内读取 GeoIP2/GeoLite2 Country 或 City 格式的 MMDB。运维在已有 Core TOML 中配置：

```toml
[network_access]
geoip_database = "/var/lib/lmm-api/GeoLite2-Country.mmdb"
```

该功能使用既有数据库和已启用的 `[console] enabled = true` 配置存储。数据库路径必须为绝对路径；文件不存在、损坏或格式不受支持时启动失败。文件在启动时载入内存，更新数据库后重启 Core 载入新版本。数据库由运维按其授权取得与更新，Core 不把请求 IP 发给第三方定位服务。

配置了本地数据库时，以本地查询为地域来源；未配置时，可使用现有可信 edge 提供的地域。两者均未提供有效记录时，地域为未知，按上述规则处理。

真实 IP 来自 TCP 连接，或通过既有 loopback + 专用 proof 验证的 edge 元数据。普通请求里的 `X-Forwarded-For`、`X-Real-IP`、`CF-Connecting-IP` 和伪造 edge 头不会改变判定。通过验证后，edge 的地址与地域写入请求上下文，专用头从下游请求移除。IPv4-mapped IPv6 按对应 IPv4 统一匹配。

## 新装与现有站点升级

新安装包含第 76 个组件 `network-access`。它只新增固定键写入函数和不可变升级回执，不自动启用地域限制。用户白名单复用既有用户设置字段。

现有站点使用既有 `upgrade-db` 离线流程，过程名为 `core-network-access-v1`，仅接受精确的 75 组件前置合同并升级至 76 组件。前置与目标摘要固定在 `services/core/src/core_schema/upgrade/network_access.rs` 和 `network_access_target.json`；请求绑定数据库身份、安装者、所有者、运行角色和唯一升级 ID。旧版本先按其既定升级链推进，不能把任意 schema 当作前置版本。

升级沿用停止服务写入、备份和安装者/运行角色分离要求；相同请求和升级 ID 可回读原回执。正常启动只校验合同，不修改数据库。运行角色只获得 `core_console.write_ip_access_rules(text)` 的执行权限，不获得 options 表的直接写权限。

管理接口的 `capabilities.writable_keys` 会包含 `IPAccessRoutingRules`。单项保存、批量保存及预校验使用同一规则验证；批量比较保存沿用 `expected_values`，冲突不部分写入。保存和审计在同一事务提交。

## 用户白名单 API

这两个浏览器接口要求用户会话，并沿用已有 Origin 校验与私有响应头：

```http
GET /api/user/self/ip-allowlist
PUT /api/user/self/ip-allowlist
Content-Type: application/json
```

PUT 请求示例：

```json
{"allow_ips":["203.0.113.7","2001:db8:1234::/48"]}
```

成功响应形状：

```json
{
  "success": true,
  "data": {
    "allow_ips": ["203.0.113.7", "2001:db8:1234::/48"],
    "client_ip": "203.0.113.7"
  }
}
```

名单最多 128 项，服务端校验、规范化网络地址并去重。单 IP 不需要 CIDR 后缀；不接受域名、端口或任意字符串。当前 IP 由服务器识别，客户端提交的额外字段会被拒绝。

保存非空名单前，Core 要求新名单包含当前来源 IP。这样可以阻止当前会话把自己立即锁在外面。界面提供“加入当前 IP”；公网 IP 会变化的连接应配置实际使用的出口网段。清空名单并保存会解除用户级限制。

名单存储在 `core_identity.user_profiles.setting.ip_allowlist`。写入时锁定该用户记录，仅更新这个字段，并记录审计事件，不覆盖语言、显示偏好等其他设置。

| 设置接口错误码 | 含义 |
| --- | --- |
| `USER_IP_ALLOWLIST_INVALID` | 地址格式、CIDR 或数量不合法 |
| `USER_IP_ALLOWLIST_LOCKOUT` | 新的非空名单未包含当前来源 IP |
| `USER_IP_REJECTED` | 请求来源被已有用户规则拒绝 |

登录、模型和原生身份接口保留各自现有错误响应格式。平台主动拒绝返回 HTTP 403 / `IP_ACCESS_ROUTE_REJECTED`；策略读取、解析或地域查询失败返回 HTTP 503 / `POLICY_UNAVAILABLE`。拒绝响应不缓存。

## 依赖与自测入口

地域读取复用 [maxminddb 0.33.0](https://github.com/oschwald/maxminddb-rust)，ISC 许可，职责仅为读取 MaxMind DB；上游仓库于 2026-11-17 查得 280 stars。它支持本地文件及选择性国家字段解码，符合本功能的离线定位需求。既有 IP/CIDR 解析继续使用 `ipnet`，zconv 等已有依赖保持原用途。

定向检查：

```sh
cargo test --manifest-path services/core/Cargo.toml --lib network_access::tests -j 1 -- --test-threads=1
cargo test --manifest-path services/core/Cargo.toml --lib user_ip_allowlist -j 1 -- --test-threads=1
cargo test --manifest-path services/core/Cargo.toml --test network_access_options -j 1 -- --test-threads=1
cargo test --manifest-path services/core/Cargo.toml --lib network_access76 -j 1 -- --test-threads=1
```

数据库测试要求独立 PostgreSQL 测试实例，并由 SQLx 创建各测试数据库。不要使用业务数据库运行这些测试。平台测试使用仓库附带的 MaxMind 合成测试数据库，来源和 MIT 许可见 `services/core/tests/fixtures/geoip/`。

本页定义配置和行为；本轮实际执行结果统一记入 [架构进度](progress.md)。


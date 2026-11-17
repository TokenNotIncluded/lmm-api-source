# Rust Core 依赖审查

zconv 固定版本更新于 2026-11-15；sysinfo、rand、psl 和 jsonwebtoken 更新于 2026-11-17，maxminddb 新增审查于同日，其余版本审查日期为 2026-11-13。范围为 `services/core/Cargo.toml` 的运行、构建及开发直接依赖；当前共有 64 个不同 crate、66 条声明（SQLx、TOML 同时用于运行和测试）。版本以当前 `Cargo.lock` 为准，统一构建与业务回归尚未完成。不是传递依赖全量审计、安全漏洞扫描或性能测试。

按用户决定，结合所属领域的成熟度判断，不设固定星数线。表中星数来自 GitHub REST `GET /repos/{owner}/{repo}`，属于上游仓库，不是单个 crate；下载数来自 crates.io `GET /api/v1/crates/{name}`，只作采用程度参考，不证明正确性。未发现这些上游被标为 archived。采用度数据采样于 2026-11-12；新增 webpki-root-certs 采样于 2026-11-13。稳定版本按注册表更新，受兼容约束的例外单列。

## 编码与文件锁

- 原生用量与历史财务 CSV 编码改用 `csv = 1.4.0`；保留原字段白名单、空值、公式防护、全字段引号、CRLF 和 32 MiB 硬上限。原生导出复用 writer；最终 flush 失败必须返回错误。
- 删除直接依赖 `fs2 = 0.4.3`，RPC socket 的排他非阻塞文件锁使用当前 Rust 1.100.0 的 `std::fs::File::try_lock`；保留文件句柄生命周期、socket inode 和目录权限规则。

## 需要单列的结论

- **zconv 是用户开发并指定最高优先级的核心依赖。** 保留固定版本，不以星数、下载量或发布时间作为替换理由；优先验证并修复它的协议转换行为。
- **zconv 0.4.0 适配范围**：Core 区分原始思考与生成摘要，保留 Responses 的两个索引空间，并仅对已验证 Gemini 来源的原生历史使用摘要分类。无法无损表示的跨协议推理、Anthropic 供应商工具块、引用与扩展用量会显式返回不支持；供应商工具不会触发本地工具执行。现有 Responses 原生身份、annotation 与 encrypted 内容边界保持不变。zconv 新增的资源编解码器不自动增加 Core HTTP 路由；本轮源码资格测试也不替代独立业务验收。
- 已检查的专用转换候选：[`llm_adapter`](https://github.com/darkskygit/llm_adapter) 3 星、[`gproxy-protocol`](https://github.com/LeenHawk/gproxy-protocol) 3 星、[`llm-relay`](https://github.com/okhsunrog/llm-relay) 2 星；均不能仅凭存在就认定比 zconv 更成熟。[`genai`](https://github.com/jeremychone/rust-genai) 940 星，但其定位是多供应商客户端，尚未做本项目等价验证。
- **protoc-bin-vendored** 虽仅 14 星，但自 2020 年发布，累计下载 35114960 次，职责仅为构建期提供 protoc；保留跨平台可复现构建用途，不把它误算为线上网络能力。
- **gjson** 上游 250 星，crate 自 2019 年发布、累计下载 2966141 次，当前版本发布于 2022 年。继续用于既有 GJSON 路径、查询与类型转换兼容；普通 JSON pointer 不能等价替换。保留兼容用途不表示维护活跃或安全审计通过。
- **oxide-auth** 当前使用固定 Git revision `0e3ef86d924aa5546dc3275d1d7269ea63edfaf1` 的 PKCE 扩展，不是 crates.io 发布包。已有 OAuth 服务端边界和冻结历史执行器依赖它；本轮保留，不能用 OAuth 客户端库的 challenge 生成函数声称替代完整服务端校验。若要求所有来源必须为 crates.io，这项仍需另行处理。
- **WebAuthn 兼容例外**：`webauthn-rs`、`webauthn-rs-core` 和测试认证器保留 0.5.5。0.6.2 所依赖的 crypto-glue 0.2.1/0.2.2 请求了稳定 RSA 0.10 已移除的 `sha2` feature，实际依赖解析失败；升级待上游依赖声明兼容后处理。
- RustCrypto 的算法 crate、rust-num 的精确计算、Hyper 的 body/适配 crate，按各自专用职责和生态采用保留；不为提高仓库星数改用会引入额外领域的综合库。
- 不使用 `rust_decimal` 替代任意精确有理数，不以进程内限流替代 PostgreSQL 的共享固定窗口；两者都会改变现有行为。

## 逐项结果

“保留”表示本轮按用途与成熟度不替换，不等于对整个库完成独立安全或业务验收。构建与测试库与运行依赖分列。

| crate | 固定版本 | 用途 | 上游星数 | 累计下载 | 决定 |
| --- | --- | --- | ---: | ---: | --- |
| [`zconv`](https://crates.io/crates/zconv) | `0.4.0` | 跨模型协议请求与响应转换 | [1](https://github.com/LIghtJUNction/zconv) | 1022 | 用户自研，最高优先级保留 |
| [`csv`](https://crates.io/crates/csv) | `1.4.0` | CSV 编解码 | [1968](https://github.com/BurntSushi/rust-csv) | 290033505 | 新增，替换手写编码 |
| [`lettre`](https://crates.io/crates/lettre) | `0.11.23` | 邮件构造与 SMTP | [2280](https://github.com/lettre/lettre) | 23934841 | 保留 |
| [`aes-gcm`](https://crates.io/crates/aes-gcm) | `0.11.1` | AES-GCM 认证加密 | [975](https://github.com/RustCrypto/AEADs) | 209738488 | 保留 |
| [`argon2`](https://crates.io/crates/argon2) | `0.6.0` | Argon2 密码哈希 | [920](https://github.com/RustCrypto/password-hashes/tree/master/argon2) | 75029494 | 保留 |
| [`base64`](https://crates.io/crates/base64) | `0.23.1` | Base64 编解码 | [743](https://github.com/marshallpierce/rust-base64) | 1949079921 | 保留 |
| [`bcrypt`](https://crates.io/crates/bcrypt) | `0.19.3` | 旧 bcrypt 密码验证 | [380](https://github.com/Keats/rust-bcrypt) | 24208037 | 保留 |
| [`chrono`](https://crates.io/crates/chrono) | `0.4.45` | 日期、时区与日历 | [3957](https://github.com/chronotope/chrono) | 972202683 | 保留 |
| [`hkdf`](https://crates.io/crates/hkdf) | `0.13.0` | 密钥派生 | [111](https://github.com/RustCrypto/KDFs/) | 345920424 | 保留 |
| [`hmac`](https://crates.io/crates/hmac) | `0.13.0` | HMAC 消息认证 | [380](https://github.com/RustCrypto/MACs) | 746719701 | 保留 |
| [`md-5`](https://crates.io/crates/md-5) | `0.11.0` | 旧支付签名兼容 | [2312](https://github.com/RustCrypto/hashes) | 458472931 | 保留 |
| [`form_urlencoded`](https://crates.io/crates/form_urlencoded) | `1.2.2` | URL 表单编解码 | [1589](https://github.com/servo/rust-url) | 976259596 | 保留 |
| [`rsa`](https://crates.io/crates/rsa) | `0.10.0` | 旧支付 RSA 验签 | [678](https://github.com/RustCrypto/RSA) | 270428346 | 保留 |
| [`jsonwebtoken`](https://crates.io/crates/jsonwebtoken) | `12.0.0` | JWT/JWS | [2106](https://github.com/Keats/jsonwebtoken) | 241735568 | 更新固定版本，适配可失败的密钥构造 |
| [`jsonschema`](https://crates.io/crates/jsonschema) | `0.61.2` | JSON Schema 校验 | [836](https://github.com/Stranger6667/jsonschema) | 120858273 | 保留 |
| [`num-bigint`](https://crates.io/crates/num-bigint) | `0.5.1` | 精确大整数 | [616](https://github.com/rust-num/num-bigint) | 740389035 | 保留 |
| [`num-rational`](https://crates.io/crates/num-rational) | `0.4.2` | 精确有理数 | [154](https://github.com/rust-num/num-rational) | 463727902 | 保留 |
| [`oxide-auth`](https://crates.io/crates/oxide-auth) | `0.6.1` | OAuth 服务端 PKCE 扩展 | [787](https://github.com/197g/oxide-auth) | 3297877 | 保留固定 Git 版本，非 crates.io |
| [`axum`](https://crates.io/crates/axum) | `0.8.9` | HTTP 应用路由 | [27812](https://github.com/tokio-rs/axum) | 585789796 | 保留 |
| [`tokio-tungstenite`](https://crates.io/crates/tokio-tungstenite) | `0.30.0` | 异步 WebSocket 帧 | [2520](https://github.com/snapview/tokio-tungstenite) | 356757171 | 保留 |
| [`futures-util`](https://crates.io/crates/futures-util) | `0.3.34` | Future/Stream 组合 | [5943](https://github.com/rust-lang/futures-rs) | 1124698061 | 保留 |
| [`http-body`](https://crates.io/crates/http-body) | `1.1.0` | HTTP body 抽象 | [173](https://github.com/hyperium/http-body) | 1070716557 | 保留 |
| [`http-body-util`](https://crates.io/crates/http-body-util) | `0.1.5` | HTTP body 适配 | [173](https://github.com/hyperium/http-body) | 611650431 | 保留 |
| [`hyper`](https://crates.io/crates/hyper) | `1.12.0` | HTTP 协议连接 | [16425](https://github.com/hyperium/hyper) | 1117876681 | 保留 |
| [`hyper-util`](https://crates.io/crates/hyper-util) | `0.1.21` | Hyper 连接/运行时适配 | [182](https://github.com/hyperium/hyper-util) | 627716666 | 保留 |
| [`ipnet`](https://crates.io/crates/ipnet) | `2.12.2` | IP 网段计算 | [171](https://github.com/krisprice/ipnet) | 728326681 | 保留 |
| [`psl`](https://crates.io/crates/psl) | `2.1.256` | 公共后缀列表与域名边界 | 未采集 | 未采集 | 更新固定版本 |
| [`maxminddb`](https://crates.io/crates/maxminddb) | `0.33.0` | 进程内 MMDB 国家字段读取 | [280](https://github.com/oschwald/maxminddb-rust)（2026-11-17） | 未采集 | 新增，ISC，专用离线地域查询 |
| [`rand`](https://crates.io/crates/rand) | `0.10.4` | 随机数生成 | [2105](https://github.com/rust-random/rand) | 2155428171 | 更新固定版本 |
| [`sysinfo`](https://crates.io/crates/sysinfo) | `0.40.4` | 主机与进程资源计数（仅 system feature） | 未采集 | 未采集 | 用户指定，更新固定版本 |
| [`regex`](https://crates.io/crates/regex) | `1.13.1` | 正则匹配 | [4056](https://github.com/rust-lang/regex) | 1380838359 | 保留 |
| [`tiktoken-rs`](https://crates.io/crates/tiktoken-rs) | `0.12.1` | 旧 OpenAI BPE 预扣估算 | [411](https://github.com/zurawiki/tiktoken-rs) | 24776336 | 保留 |
| [`fancy-regex`](https://crates.io/crates/fancy-regex) | `0.20.2` | 兼容分词的扩展正则 | [646](https://github.com/fancy-regex/fancy-regex) | 301795196 | 保留 |
| [`reqwest`](https://crates.io/crates/reqwest) | `0.13.5` | HTTP 客户端传输 | [11930](https://github.com/seanmonstar/reqwest) | 903120718 | 保留 |
| [`webpki-root-certs`](https://crates.io/crates/webpki-root-certs) | `1.0.12` | Mozilla CA 证书数据 | [158](https://github.com/rustls/webpki-roots) | 143275107 | 新增，保持原 WebPKI 信任边界；证书数据许可 CDLA-Permissive-2.0 |
| [`ring`](https://crates.io/crates/ring) | `0.17.14` | 密码学原语 | [4131](https://github.com/briansmith/ring) | 895972663 | 保留 |
| [`serde`](https://crates.io/crates/serde) | `1.0.229` | 序列化抽象 | [10974](https://github.com/serde-rs/serde) | 1692169549 | 保留 |
| [`serde_json`](https://crates.io/crates/serde_json) | `1.0.154` | JSON 编解码 | [5679](https://github.com/serde-rs/json) | 1598517471 | 保留 |
| [`sha2`](https://crates.io/crates/sha2) | `0.11.1` | SHA-2 摘要 | [2312](https://github.com/RustCrypto/hashes) | 1214221305 | 保留 |
| [`sqlx`](https://crates.io/crates/sqlx) | `0.9.0` | 类型化 PostgreSQL 访问 | [17659](https://github.com/launchbadge/sqlx) | 187649319 | 保留 |
| [`tokio`](https://crates.io/crates/tokio) | `1.54.1` | 异步运行时 | [33664](https://github.com/tokio-rs/tokio) | 1181375534 | 保留 |
| [`fs2`](https://crates.io/crates/fs2) | `0.4.3` | RPC socket 排他文件锁 | [161](https://github.com/danburkert/fs2-rs) | 102837251 | 移除，改用标准库 |
| [`flate2`](https://crates.io/crates/flate2) | `1.1.11` | DEFLATE/gzip | [1132](https://github.com/rust-lang/flate2-rs) | 843396163 | 保留 |
| [`zstd`](https://crates.io/crates/zstd) | `0.14.4` | Zstandard 压缩 | [668](https://github.com/gyscos/zstd-rs) | 475586502 | 保留 |
| [`prost`](https://crates.io/crates/prost) | `0.14.6` | Protobuf 编解码 | [4809](https://github.com/tokio-rs/prost) | 705631565 | 保留 |
| [`subtle`](https://crates.io/crates/subtle) | `2.6.1` | 常数时间比较 | [314](https://github.com/dalek-cryptography/subtle) | 864804634 | 保留 |
| [`totp-rs`](https://crates.io/crates/totp-rs) | `6.0.0` | TOTP 认证码 | [281](https://github.com/constantoine/totp-rs) | 12679356 | 保留 |
| [`zeroize`](https://crates.io/crates/zeroize) | `1.9.1` | 秘密内存清零 | [690](https://github.com/RustCrypto/utils) | 873186997 | 保留 |
| [`webauthn-rs`](https://crates.io/crates/webauthn-rs) | `0.5.5` | WebAuthn 验证 | [722](https://github.com/kanidm/webauthn-rs) | 9144888 | 保留 |
| [`webauthn-rs-core`](https://crates.io/crates/webauthn-rs-core) | `0.5.5` | WebAuthn 底层协议类型 | [722](https://github.com/kanidm/webauthn-rs) | 10586963 | 保留 |
| [`tokio-stream`](https://crates.io/crates/tokio-stream) | `0.1.19` | Tokio Stream 适配 | [33664](https://github.com/tokio-rs/tokio) | 578095575 | 保留 |
| [`toml`](https://crates.io/crates/toml) | `1.2.1+spec-1.1.0` | TOML 配置编解码 | [1091](https://github.com/toml-rs/toml) | 1129232431 | 保留 |
| [`tonic`](https://crates.io/crates/tonic) | `0.14.9` | gRPC 传输 | [12543](https://github.com/hyperium/tonic) | 470373986 | 保留 |
| [`tonic-prost`](https://crates.io/crates/tonic-prost) | `0.14.9` | gRPC Protobuf codec | [12543](https://github.com/hyperium/tonic) | 146454795 | 保留 |
| [`tower`](https://crates.io/crates/tower) | `0.5.3` | Service 中间件抽象 | [4353](https://github.com/tower-rs/tower) | 831177172 | 保留 |
| [`gjson`](https://crates.io/crates/gjson) | `0.8.1` | 旧 OAuth GJSON 路径语义 | [250](https://github.com/tidwall/gjson.rs) | 2966141 | 保留 |
| [`tonic-prost-build`](https://crates.io/crates/tonic-prost-build) | `0.14.9` | gRPC/Protobuf 代码生成（构建） | [12543](https://github.com/hyperium/tonic) | 78580758 | 保留 |
| [`protoc-bin-vendored`](https://crates.io/crates/protoc-bin-vendored) | `3.3.0` | 构建期固定 protoc 二进制定位（构建） | [14](https://github.com/stepancheg/rust-protoc-bin-vendored/) | 35114960 | 保留 |
| [`webauthn-authenticator-rs`](https://crates.io/crates/webauthn-authenticator-rs) | `0.5.5` | 测试用软件认证器（测试） | [722](https://github.com/kanidm/webauthn-rs) | 4075428 | 保留 |

## 基础库接入与验证

- 十六进制使用用户指定的 [`zhex = 0.1.3`](https://crates.io/crates/zhex/0.1.3)，上游为 [LIghtJUNction/zhex](https://github.com/LIghtJUNction/zhex)，MIT，无传递依赖。已移除 hex 直接依赖；固定缓冲区解码直接调用 zhex，`hex_codec` 只为 String/Vec 返回值分配缓冲区，保留入口长度、大小写和错误分类。
- 百分号解码使用 [percent-encoding 2.3.2](https://docs.rs/percent-encoding/2.3.2/percent_encoding/)，HTML 转义使用 [html-escape 0.3.1](https://docs.rs/html-escape/0.3.1/html_escape/)，保留原 HTML 实体表示。
- 用户指定的 [sysinfo](https://github.com/GuillaumeGomez/sysinfo) 0.40.4（仅 system feature）承接资源计数，保留 250ms 快照、整机 CPU 归一化和权限下的进程识别，不收集命令行或环境。
- 用户指定的 [zclap](https://github.com/LIghtJUNction/zclap) 0.1.4 以 parse_os 保留非 UTF-8 绝对路径及原命令互斥；[tokio-util](https://github.com/tokio-rs/tokio) 0.7.20（rt feature）的 CancellationToken 替代 watch 手写实现，资金收尾仍不随上游取消。
- [serde_json RawValue](https://docs.rs/serde_json/latest/serde_json/value/struct.RawValue.html) 配合 Seed/Visitor 替代 receipt Scan 词法扫描，保留逐层重复键拒绝、深度、数字拼写和签名字节；其余业务验证仍由 Core 负责。

2026-11-17 的依赖更新见 [1362b3ac](https://github.com/TokenNotIncluded/lmm-api-source/commit/1362b3ac2c741c83cf955f5880d62bf28ab7ce4d)：适配 jsonwebtoken 12 的可失败密钥构造，并显式拒绝无效 HMAC 解码密钥和空密钥。固定 74 组件渠道候选的 release 构建与标准发布包验证通过；隔离 PostgreSQL 批量配置 3 项通过，旧 vault 测试为 2 项通过、1 项旧夹具失败。规范恢复替代用例及 JWT、PSL、sysinfo、relay/zconv 定向库回归尚未启动，不能据局部渠道测试认定依赖行为回归通过；本轮未部署。当前阻塞和证据入口见[新架构进度](progress.md)。上述版本更新未重新采集星数或下载数，表中既有采用度数据仍对应原采样日期。

2026-11-12 开发自测中，CSV 与用量导出定向测试通过；internal_rpc 8 项通过、1 项因未提供 `DATABASE_URL` 失败。zhex 接入后的 137 项普通/本地异步测试通过，普通 Clippy 通过但有 141 条既有警告；依赖审查轮次的严格 Clippy 在未改位置报告 70 项错误。原始失败保留在对应 Git 历史，不将换库或局部自测视为独立业务验收、真实 PostgreSQL、性能、发布或部署验证。

SSE 解析和冻结旧计价表达式解释器仍待评估，不能称全部手写基础能力已移除。

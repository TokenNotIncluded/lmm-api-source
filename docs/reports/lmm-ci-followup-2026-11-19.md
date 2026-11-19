# LMM API 后端修复、CI/CD 与 0.1.21 发布记录

日期：2026-11-19（Asia/Taipei）。**[v0.1.21 源码预发布](https://github.com/TokenNotIncluded/lmm-api-source/releases/tag/v0.1.21)已于 20:34:45 发布并完成附件回读验证。** 本记录区分开发者自测、GitHub 工作流和实际发布；当前冻结候选的独立业务验收尚未执行。持续状态只维护在[新架构进度](../progress.md)，完整回执摘要见[结构化证据](lmm-ci-followup-2026-11-19.evidence.json)。

## 已提交的修复

本轮已通过 GitHub 提交三批修复。Go 环境隔离见 [`f8a2868e`](https://github.com/TokenNotIncluded/lmm-api-source/commit/f8a2868ebc61fb974eb1e8eb4bd90a0e8b529ee6)，主 CI 的 PostgreSQL 18 宿主工具见 [`d40e8a21`](https://github.com/TokenNotIncluded/lmm-api-source/commit/d40e8a21116aeabbff1a3a9c45cda58155010000)，Core 与测试夹具、MK 工作流见 [`9d50a054`](https://github.com/TokenNotIncluded/lmm-api-source/commit/9d50a05465d2680904ebdbb041b3e877a04e1448)。源工作副本已同步，未覆盖原有开发工作区。

Core 修复了两类实际行为问题。浏览器授权列表只在配置了 managed 模块时读取可选管理表，使 CLI 单独安装时无需获得额外表权限。流式响应的补充 `cost_details` 从内容转换输入中移除，原始回执仍由 Core 解析；收费只取实际 `usage.cost`，缺失、零值、负数、字符串和精度限制仍按原合同处理。

支付测试失败来自共享夹具缺少现有订阅 schema。补入 `subscription_epay_schema.sql` 和 `payment_subscription_inbox` 的既有读写权限后，真实 Go 扩展能够通过启动检查，两个 Core／Go 崩溃恢复用例均通过。生产启动守卫、进程身份核验、故障窗口、资金及重放断言保持原样。

## 实际验证结果

Rust 定向集合共取得 **80 个不同用例的通过结果**，另有 1 个原有 ignored 用例。这个数字按不同用例计算，不累计重试，也不代表完整后端 CI 已通过。

| 范围 | 实际结果 | 验证内容与边界 |
| --- | --- | --- |
| 身份、权限、邮件和 Passkey | 11 通过 | OAuth 可选模块、调用者身份、撤权、邮件令牌、RP 与来源限制等 |
| 协议与模型 | 64 通过，1 原有 ignored | lib 10、完整 model_http 32、relay 22；使用合成本地上游 |
| 钱包、推荐奖励和旧安装器拒绝 | 3 通过 | 钱包与奖励生命周期；当前安装器拒绝过期 29→31 请求并保持数据不变 |
| 支付真实进程 | 2 通过 | 原两个精确用例各执行一次，测试命令耗时 83.613／77.442 秒 |
| Go 连接环境隔离 | 5 个顶层测试通过 | 含子测试共 46 个 pass 事件；注入 6 个合成 PG 环境变量，启用 race |
| CI 财务夹具 | 2 通过 | 干净 Ubuntu 24 环境安装 PG18 后断网执行；无系统共享集群 |

每组执行绑定实际源码集合、文件哈希及二进制。普通 Core ELF 先复制到不可变私有路径，再供测试复用，避免 Cargo 后续测试构建覆盖同名目标。支付补丁只改变一个测试文件，因此只重建两个支付目标；原 Core、Go 和 bootstrap 保留其真实来源标识。源码分组存在重叠，837／54／255／1414／1 的分组计数不能相加当作唯一文件数。

两次夹具问题的首轮失败均保留。relay 的一个用例因驱动遗漏必需的冻结 Go 程序路径失败，补齐环境后只复验该用例一次；其余通过用例未重跑。operations 的拒绝行为和数据不变性断言通过后，清理阶段遇到 PostgreSQL 锁容量不足；在新建的隔离夹具中将 `max_locks_per_transaction` 从 64 调到 256，同一 ELF 精确复验一次通过，其他隔离参数不变。该结果没有证明历史 29→31 正向升级或生产容量。

所有上述测试使用的 owned PostgreSQL 均已按身份绑定确认清理，包括原失败的协议、迁移、quota 和支付夹具。原失败日志、来源绑定和清理回执保留；支付每例还保留 7 份 Core、Go、bootstrap 子进程日志及恢复证据。

## 尚未完成的迁移准入

**Key quota 仍未闭合。** 第一项精确用例失败，另 7 项未运行。已审核目标是 69 个组件，当前冻结 Core 和实际测试数据库均为 76 个组件：原有 69 个组件的哈希不变，增加 managed-applications、affiliate-registration、oauth2-oidc、personal-tokens、subscription-creem、checkin-challenges、network-access。现有审核档案没有对应分支，故 `component_rust_source_fingerprint_mismatch` 正确拒绝继续。未更换审核 pin、关闭守卫或将当前指纹自动加入白名单；本轮未发布该未完成候选。

**Community 历史导入仍受 55／56 组件合同限制。** 当前 76 组件运行时不能直接套用这条历史导入路径。本轮未修改其 catalog 或 SQL 限制，也未将源码存在视为迁移已验收。后续需要绑定受支持的历史执行器、升级路径和实际目标合同。

Key quota 的后续实现路径已收敛。源码与保留的真实 manifest 核对表明，当前 76 二进制允许按前缀执行现有七步 69→76 suffix，七步前后 manifest 和相邻 catalog 均匹配。可以保留 reviewed69：先用绑定的历史 69 初始化执行器导入并验证重放、回滚，再升级到 76 检查配额行为。仍需补初始化执行器及分阶段 fixture 接线；这次只读核对没有执行数据库升级，也没有证明 community 的 55／56→69 桥接路径。

## CI/CD 与发布范围

MK 工作流已补齐与主 CI 相同的 PostgreSQL 18 准备步骤，时限由 25 分钟调整为 150 分钟，并串行运行会检查全局角色目录的测试。修改范围 actionlint 1.7.12 和 18 项工作流拓扑检查通过。主 CI 的 PG18 安装命令还在干净 Ubuntu 24 容器中实际验证，财务夹具 2 项通过。

远端 CI 保留独立结果。[`d40e8a21` 的 Web 与翻译任务](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44509988084)已通过；[`9d50a054` 的主 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44520459078)的 Web 和翻译也通过，完整后端尚无最终成功结论。[`9d50a054` 的 MK 运行](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44520459099)在安装阶段遇到 Ubuntu runner 镜像源 HTTP 503，退出 100，尚未进入 Cargo 或业务测试；该失败继续保留。

APT 准备脚本现只将精确的 runner `mirror+file:/etc/apt/apt-mirrors.txt` URI 改为 Ubuntu 官方 HTTPS archive，保留套件、组件、签名及未知来源。5 项配置夹具测试通过；这证明配置转换，不证明远端网络已经恢复，网络结果仍以新 CI 为准。

部署门禁已实现，并接入日常 CI。构建前读取同一源码 SHA 的 canonical main CI，要求最新 run／attempt 及其唯一聚合检查均成功；旧的成功记录不能覆盖新的失败或运行中结果。门禁只读已有结果，构建和部署都依赖它。10 项模拟 API 测试与最终 19 项工作流拓扑检查通过；未触发真实部署。具体行为见[部署说明](../../deployment/automation/README.md)。

CI/CD 修复已提交为 [`9b9d4234`](https://github.com/TokenNotIncluded/lmm-api-source/commit/9b9d4234bd4a926ecc557e076a69615943a68c37)，发布请求为 [`0c8844b0`](https://github.com/TokenNotIncluded/lmm-api-source/commit/0c8844b0ea13b67ca25fa6103b712260e8f4011a)。新门禁还通过真实 GitHub API 只读检查：当前完整 CI 为 in_progress 时，实际返回拒绝，未进入部署。

**[0.1.21 源码预发布](https://github.com/TokenNotIncluded/lmm-api-source/releases/tag/v0.1.21)已发布。** [`Qualified source release` run 44523376980](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44523376980)的 qualify 和 publish 均成功；格式、Clippy、289 项 relay、10 项 edge_access、1 项 Turnstile 测试全部通过。这 300 项发布资格自测与前述定向集合有重叠，不将两组相加。标签精确指向 `0c8844b0ea13b67ca25fa6103b712260e8f4011a`；发布期间 main 保持不变。

四个附件均已下载，文件大小和 SHA256 与 GitHub 元数据相符；`SHA256SUMS` 的三条记录、`source.json` 的仓库／版本／提交／workflow 身份，以及归档内本轮 14 个发布准备文件均已核对。源码包为 17,240,419 bytes，SHA256 `f54d7380bc1832ddf93df535c4f34865e6e1a1ecd83e75f8595257a972393d9f`。附件包括源码包、`source.json`、`core-relay-tests.log` 和 `SHA256SUMS`。

源码发布成功没有改变完整后端 CI 的真实状态。[同一提交的完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44523376933)中，Web 和翻译通过，工具安装退出 100。完整日志确认镜像 URI 替换已经生效，但 runner 访问 Ubuntu 官方 archive 时出现 TLS/select 资源错误，访问 security 仓库时连接失败。后续发布记录提交按既有并发策略取消了后端 job，聚合检查为 failure。失败安装步骤日志已保留，SHA256 为 `1b8753fd6ca08111a537c860f84f12c23846c4c01c81ff3ebd10549c7d796694`；未关闭 TLS、包签名或业务门槛。该环境阻塞与迁移准入缺口均保持未完成，源码发布资格不替代完整 CI 成功。

源码预发布与运行环境部署分别记录。本轮没有触发服务部署、生产数据库迁移或切流。后续部署先由新门禁确认同一源码提交的完整 CI 和聚合检查成功，再使用既有配置、制品验证及环境保护。完整后端、迁移准入和业务验收的剩余问题仍是生产就绪条件。

## 证据入口

以下路径相对于开发机项目目录下的 `out/lmm-ci-followup-20261119/`。表中只记录回执哈希，不公开运行凭据或私有配置；其余回执见[结构化证据](lmm-ci-followup-2026-11-19.evidence.json)。

| 证据 | SHA256 |
| --- | --- |
| `build-recovered2/build-receipt.json` | `c2ec454122ce6aae2883d4d8886905ae322e7a0fc050f9d80a84778d0b01e93a` |
| `build-payment-fixed/build-receipt.json` | `63462231e36faefbe1d5aba0643084f11256adea90e06c1f2d6e38d3fcc7f19c` |
| `rust-owned/identity-recheck/result.json` | `f7a58b0d287b820f253753586e92ce559f6c355afbf4bde17d107e6880cc9c4c` |
| `protocol-recheck/result.json` | `389b62a17af7b1129739008f903b8b40c7b4d5dd0ccbfedb8c279a37f772a03c` |
| `relay-go-environment-recheck/result.json` | `370bc6b931ece88180927f79366afe49ae0fadfab7822120ca4fd67672532c9d` |
| `migration-selftest/candidate-recovered2/result.json` | `bbd306a9c3b027a30fbece048810599b9eb63a3415f745b6fcc80f340589de19` |
| `migration-selftest/operations-locks256-recheck/result.json` | `94f3cd8aeabdf737732c6a97ecd8302c1df6d3d8c791b59c0f6f09625613141c` |
| `payment-fixed-recheck/result.json` | `37cfd23af4e3fe7875c74782c4ff3efee8f49b7194e6f9050f44e124a9628a99` |
| `rust-owned/keyquota-recheck/diagnosis/profile-mismatch.json` | `86071cbd622f1bf7a2cd9fa1d1e974003cff9f03a6033eb407567519027f4083` |

**self-tested; independent acceptance not run**

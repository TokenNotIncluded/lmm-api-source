# 原生渠道、路由、用户管理与支付配置：本轮证据

日期：2026-11-18。此记录承接[渠道批量路由与模型估算器](lmm-channel-routing-2026-11-18.md)，只汇总本轮代码、验证及发布状态；早期记录按其当时的版本保留。

**当前结论：已部署应用版本 6fa1b905 的 Core、Extensions、Web 已通过最终发布流程及 CI 运行态验收；渠道超时修复、native 数据库 76、管理反代及 EPay 5004 禁用配置导入均已完成。** [三组件发布 run 44315659498](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498)／[部署 job 134848523976](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498/job/134848523976) 于 13:52:42 UTC（21:52:42 Asia/Taipei）成功完成。Core `check-db`、三组件 health、Core／Extensions 实际运行文件摘要与 manifest、served HTML 与 Web 产物比对全部通过。5004 三阶段运维回执、72392577 上一批发布与 75c9d482 首次 Core 摘要分别保留为历史证据。服务器备份及数据库备份的隔离恢复校验已完成；开发机第二份完整备份复制仍未完成。官方 Playwright 扩展桥已连接，当前等待用户在独立后台标签完成管理员登录；渠道超时页面读回与真实管理员浏览器 E2E 仍待继续。旧版主域名保持原部署。

## 当前状态

| 范围 | 已实现与版本 | 验证／部署状态 | 下一项完成证据 |
| --- | --- | --- | --- |
| 渠道草稿与模型故障切换 | [7a67cd0e](https://github.com/TokenNotIncluded/lmm-api-source/commit/7a67cd0e3cf2d1b0f5f288505d115f9e019cd46c)：版本绑定草稿、重试次数、原因及发送后重放可视化 | 已随本轮发布；组件定向自测通过 | 真实管理员编辑、保存、冲突恢复与重新读取 |
| 用户管理界面与 Core 合同 | 列表／详情团队字段、筛选及可访问分组读取／保存已接通 | Web／Core 已发布，native 管理反代已上线；旧 Core 浏览器错误保留为修复前证据 | 管理员登录后只读复验列表、详情与可访问分组；本轮不保存用户分组 |
| Pancake 独立绑定 | [6fd21f21](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fd21f219ab3d88b23ef8b77468b734a186ab9b6)，已纳入最新交付 | Go／Web 定向检查通过；管理员保存与真实支付交易仍待验收 | 读取、保存、重新读取的实际回执 |
| Core 路由、模型目录与用户合同 | [c1d93906](https://github.com/TokenNotIncluded/lmm-api-source/commit/c1d93906ec6fa02cc15e7081428cea68906b9560)，格式修复 [75c9d482](https://github.com/TokenNotIncluded/lmm-api-source/commit/75c9d482ea03885db0a2bd4376f06f04f3e9f51a) | 18 项定向 lib、1 项 vault、2 项隔离 schema 升级测试通过；正式数据库已升级至 76 | 登录态业务验收 |
| 渠道超时默认值与发布错误提示 | [6fa1b905](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e) 已进入 main 并部署；Core 真实下限和专用 409、Web 新目标默认值与提示已接通 | Core 3/3；Web 5 文件 77/77、tsgo／修改范围 lint／格式 exit 0；三组件发布与 CI 运行态验收成功 | 管理员登录后的页面读回与既定操作 |
| 最终 Core／Extensions／Web 发布 | 已部署应用 [6fa1b905](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e) | [run 44315659498](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498)／[job 134848523976](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498/job/134848523976) 全成功；数据库兼容、health、实际进程文件及 served HTML 均通过校验 | 真实管理员验收；开发机 SSH 补充独立回读仍受阻 |
| 上一批三组件发布（历史阶段） | 72392577 | [run 44305784095](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095)／[job 134813816947](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095/job/134813816947) 成功，随后完成 5004 导入 | 保留该阶段自己的运行态回执 |
| 第一批 Web／Extensions 发布（历史阶段） | 格式修复 [3b140f2c](https://github.com/TokenNotIncluded/lmm-api-source/commit/3b140f2c6ab7b837b29de27233603b4bcd2a192c) | [run 44296422533](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44296422533) 成功，后由最终三组件流程接续 | 历史回执保留 |
| 第一批 Core 发布（历史阶段） | 固定 75c9d482，具名用户级单元完成构建 | 两次客户端重启中断保留；最终构建／打包／校验均 exit 0，编译 6m57s，运行摘要已回读 | `release-build-persistent.json`；该阶段摘要单独保留 |
| EPay／LINUX DO Credit | owner 配置 CAS、私有密钥持久化、共享运行版本与钱包最低额合同自 72392577 发布，本次 6fa1b905 继续保留 | Core 16/16、bin 检查 0、Go／Web 检查通过；5004 已经 stage／prepare／activate 三阶段导入，私有 base 已切换、health 通过，仍保持禁用 | 真实 owner 安全投影、保存、冲突与版本回读 |
| native 管理反向代理 | 精确方法、正整数 ID 与路径白名单；只装入 native vhost | 真实 Nginx loopback 2/2，含 14 个拒绝场景；实际部署与公网匿名拒绝边界已回读 | 已认证请求与页面操作 |
| 备份与恢复 | 服务器保留数据库与运行时备份；数据库备份完成隔离恢复校验 | 开发机第二份完整备份复制因 SSH 握手阻塞未完成 | 完整复制与摘要匹配回执 |
| 真实管理员浏览器验收 | 官方 Playwright 扩展桥已连接，真实后台页面操作已开始 | 独立后台标签稳定在登录页，等待用户完成管理员登录；业务页面验收尚未通过 | 渠道、用户列表／详情与支付配置的实际页面回执；用户分组本轮只读 |

## 行为与验证边界

模型故障切换明确区分尝试次数与重试次数：`max_attempts` 包含首次请求，1 表示不重试；支持后端现有的 `connect`、`timeout`、`rate_limited`、`server_error` 分类。发送后重放默认仍为关闭，界面提示重复执行和重复费用风险；路由配置不能单独绕过请求级重放限制。模型其余字段、JSON 草稿、版本与冲突处理合同继续保留。实现及用例见 [native-failover-fields.tsx](../../apps/web/src/features/channels/components/native-failover-fields.tsx) 与[组件测试](../../apps/web/src/features/channels/components/native-failover-fields.test.tsx)。

用户管理修复直接覆盖浏览器与 Core 的合同断裂：列表／详情补齐团队字段和筛选，接通用户可访问分组的读取与保存。保存校验会话权限和版本，分组修改、认证版本变化及审计在同一事务内完成；部署授权与运行时权限检查同步加入所需的最小列权限。4 项真实隔离 PostgreSQL 加 router HTTP 测试使用标准运行角色，覆盖成员状态、浏览器请求合同、冲突、审计失败回滚及列权限边界，见[用户管理测试](../../services/core/src/identity/admin_users/access_groups_tests.rs)。这些结果尚未覆盖生产浏览器中的保存操作。

路由修复使渠道密钥池的选择、故障切换和在途请求使用一致的配置版本，保留 Responses 续接绑定。模型目录读取补齐分页与游标处理，并验证跨页失败不会返回部分成功。证据分别位于[密钥池测试](../../services/core/src/routing/key_pool_tests.rs)、[路由持久化测试](../../services/core/src/model_http/route_failover_tests.rs)和[目录测试](../../services/core/src/model_http/channel_catalog_tests.rs)。

渠道超时默认值与发布错误提示已在 [6fa1b905](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e) 修复，通过定向验证并完成三组件发布。Core 配置读取返回 `data.model_limits.total_timeout_ms`；低于实际运行下限的启用 upstream 目标在预览、发布和批量修改时返回专用 409 `CHANNEL_TARGET_TIMEOUT_TOO_SHORT`，附 `data.required_min_timeout_ms`。Web 新目标采用 `max(30000, actual)`，既有值保留。修复不改变目标原有准入范围、重试或执行 deadline；请求执行仍使用全局 `ModelLimits.total_timeout`。实现见[发布验证](../../services/core/src/channel_http/control.rs)、[HTTP 合同测试](../../services/core/tests/channel_timeout_contract.rs)与[输入请求执行](../../services/core/src/model_http/input_api.rs)。

EPay 管理保存通过 Core 维护单一有效配置版本。HTTP 与只读 RPC 返回同一安全投影；仅 owner 可以修改没有历史资金绑定、保持禁用的 LDC EPay 连接。商户身份、币种、倍率、最低／最高金额与启用状态保持只读，空密钥输入保留原密钥。配置使用 revision 做并发比较，私有 overlay 与不可变密钥文件持久化后发布共享运行快照；每个已进入的请求固定所用快照。最终持久化确认或审计失败会返回明确的不确定状态，界面回读版本。证据见[配置实现](../../services/core/src/payments/configuration.rs)、[私有存储](../../services/core/src/payments/configuration/storage.rs)与[真实 PostgreSQL／HTTP／RPC 回归](../../services/core/src/payments/configuration/tests.rs)。

钱包最低额按新订单的最终实付金额检查，缺省 `min_amount_minor=1` 兼容既有配置。实际导入的 LDC 渠道 5004 保留 `min_amount_minor=10000`、`credits_per_minor=500` 与 `enabled=false`；配置导入已完成，启用状态保持关闭。已有钱包订单的幂等恢复、结账和凭证处理继续使用原义务，订阅价格不套用钱包最低额。新用例覆盖“钱包最低额 10000、订阅价格 125”仍可创建订阅的合同；禁用连接停止新订单／价格准入，已有义务继续恢复，见[订阅回归](../../services/core/src/subscriptions/external_epay_tests.rs)。

## 已完成测试回执

### 渠道与用户管理第一批回执

下表按实际日志逐项计数，重复运行不重复计入。18 项 lib 测试由 5 + 5 + 1 + 2 + 1 + 4 组成；新增的第 5 项密钥池用例为 `key_pool_group_alternatives_advance_each_channel_once_per_request`，覆盖多授权组候选对共享轮询位置的推进。前一轮 `final-*` 回执为 17 项，后一轮 `exact-*` 回执为 18 项；两轮不能重复累计。这是定向回归，不是完整 Core 测试集。

| 检查 | 结果 | 开发机证据文件 |
| --- | --- | --- |
| lib 编译 | exit 0 | `exact-core-lib-build.log` |
| 密钥池与配置版本 | 5 通过 | `exact-key-pool.log` |
| 模型目录分页／边界 | 5 通过 | `exact-channel-catalog.log` |
| 供应商模型目录解析 | 1 通过 | `exact-provider-parser.log` |
| Responses turn 授权／预留 | 2 通过 | `exact-responses-turn.log` |
| Responses 原生目标与续接绑定 | 1 通过 | `exact-responses-pinning.log` |
| 用户合同／分组写入／权限 | 4 通过 | `exact-admin-user-access.log` |
| vault 启用状态、授权与版本 CAS | 1 通过 | `final-vault-polling.log` |
| 74→75 精确 schema 升级与重放 | 1 通过，隔离 PostgreSQL | `checkin75_exact_upgrade_preserves_data_acl_and_replays.log` |
| 75→76 精确 schema 升级与重放 | 1 通过，隔离 PostgreSQL | `network_access76_exact_upgrade_preserves_data_acl_and_replays.log` |

本轮 Core 日志目录为开发机 `~/.local/state/lmm-api/channel-routing-20261118/`；`exact-core-checks.json` 保存最终 18 项 lib 的命令、退出码及完成时间，`final-core-checks.json` 保存前一轮 17 项 lib 与 vault 回执，`schema74-to-76-checks.json` 保存两项升级测试回执。上述 schema 用例验证隔离库的数据与权限保留；正式 native 数据库随后于本轮完成 74→75→76 升级；线上完成依据为下节的部署回执，不能用隔离测试替代。

### EPay 与接入最终回执

EPay 最终 Core 用例为 6 + 4 + 2 + 1 + 1 + 1 + 1，共 16 项。`epay-core-min-final-checks.json` 记录全部命令、退出码与完成时间；它引用已成功的 lib 编译回执，并确认 13 个 Rust 文件与 payments.proto 共 14 个输入在编译、串行定向回归和主程序检查期间 SHA-256 未变。

| 检查 | 结果 | 开发机证据文件／源码入口 |
| --- | --- | --- |
| 配置、owner／CAS、审计失败、重启、禁用准入与钱包最低额 | 6/6；真实隔离 PostgreSQL，含 HTTP／RPC 与标准运行角色 | `epay-final-configuration.log` |
| Payments RPC 授权 | 4/4 | `epay-final-rpc-authority.log` |
| EPay 供应商协议 | 2/2 | `epay-final-provider-protocol.log` |
| 原义务凭证在取消／改价／换 key 后恢复 | 1/1 | `epay-final-obligation-recovery.log` |
| 钱包与 EPay 共享购买限额 | 1/1 | `epay-final-shared-purchase-limit.log` |
| 管理员 EPay 价格读取 | 1/1 | `epay-final-admin-price-read.log` |
| 冻结订阅绑定 | 1/1 | `epay-final-frozen-subscription-binding.log` |
| Core 主程序装配 | `cargo check --bin lmm-core` exit 0，56.4 秒 | `epay-final-core-bin-check.log` |
| 精确 Rust 格式与 diff 检查 | 两项均 exit 0 | `epay-source-format-checks.json` |
| Go 定向命令 | 4 个包均 exit 0；`internal/app` 明确为 `[no tests to run]`，其余 3 个包执行匹配用例 | `epay-final-go.log` |
| EPay Web 保存、冲突、不确定写入、锁定历史、权限与会话切换 | 6/6 | `epay-final-web.log` |
| Web 类型与 lint | tsgo／oxlint 均 exit 0 | 本轮定向命令回执；`epay-min-typecheck.log` |
| Nginx 方法、路径、会话与请求体转发 | 2/2，真实本机 Nginx 与 loopback HTTP 上游；第二项包含 14 个拒绝场景 | [测试脚本](../../deployment/native/test_management_routes.py)、本轮执行回执 |
| Nginx native include 部署 | exit 0；实际匿名状态见下节 | `nginx-management-deployment.json` |
| 独立公网匿名入口 | 登录页 200，管理入口按各自权限合同拒绝 | `final-native-ingress.json` |

Nginx 用例可通过 `python3 deployment/native/test_management_routes.py` 在安装了 Nginx 的开发环境重现。脚本在临时目录、随机 loopback 端口与合成会话下验证方法、原 URI、Authorization 和请求体；14 个拒绝场景覆盖错误方法、缺失／非法 ID、相邻路径及未开放入口。[模板](../../deployment/native/templates/native-management-locations.conf)与测试均存在于 72392577。生产匿名回读与本地用例分别记录。

前端用户管理与故障切换组件合并定向运行 23 项通过，修改范围 lint 为 0 错误、0 警告。此前 EPay 文案通过翻译 helper 应用七语言，该次同步变更为 0，missing／extra 均为 0，新增 key 均非空。该次全站检查仍有既存 untranslated 计数：fr 52、ja 116、ru 117、vi 52、zh-CN 10；本轮结果不表示全站翻译已全部完成。

### 渠道超时修复回执（6fa1b905）

修复源码已进入 main，已部署应用为 6fa1b905；[三组件发布 run 44315659498](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498) 及其 [部署 job 134848523976](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498/job/134848523976) 均成功。下列开发验证与[最终线上发布回执](#最终发布与-native-管理反代)分别记录。

| 检查 | 实际结果 | 开发机证据 |
| --- | --- | --- |
| Core 超时 HTTP／PostgreSQL 合同 | 3/3；编译 6m19s，用例 4.09s，exit 0；5 个输入的前后 SHA-256 相同 | `channel-timeout-contract-checks.json` |
| Web 超时与既有编辑合同 | 5 个测试文件，77/77，全部 exit 0 | `timeout-web-tests.json` 及其引用日志 |
| 完整 Web 类型检查、修改范围 lint／格式 | 均 exit 0 | `timeout-web-gates.json`、修改范围格式命令回执 |
| 新增文案 | 4 个 key 的七语言内容完整 | 本轮翻译 helper 与同步回执 |

Core 回归覆盖实际下限读取、保留已有值、预览／发布／批量错误、真实 preview 确认摘要、满足下限后的保存，以及禁用 upstream 草稿和重新启用检查。Web 验证覆盖新目标默认值与现有编辑数据的保留。首次 Web lint 曾有两条 curly 错误，修复后通过；`timeout-web-gates.json` 保留 `initial_lint_exit_code=1`，原失败日志继续保留。新 key 的完整性不改变前述全站既存 untranslated 计数。

## 保留的失败与中断

| 历史尝试 | 当时结果 | 后续证据 |
| --- | --- | --- |
| 超时修复首次 Web lint | 两条 curly 错误，exit 1；`timeout-web-gates.json` 保留首次失败码，原日志保留 | 修复后修改范围 lint／格式及完整 tsgo 均 exit 0；Web 5 文件 77/77 |
| 初次密钥池定向测试 | `key-pool-lib-first.log`：3 通过、1 失败 | 最终密钥池 5 项通过；原失败日志保留 |
| 初次模型目录测试 | `catalogue-tests.log`：1 通过、4 失败 | 修复后最终同范围 5 项通过；原失败日志保留 |
| 首次 Core release 构建 | `release-build.json` 标记客户端重启中断，未观察到完成 | 不能记为构建通过 |
| 第二次 Core release 构建 | `release-build-resumed.json` 无完成回执，客户端重启再次中断 | 改由具名单元承载，最终完成回执已确认 exit 0；原中断仍单独保留 |
| 首次 EPay Core 验证 | `epay-core-checks.json` 无完成回执，客户端重启中断 | 原记录保留；改由具名单元完成后续验证 |
| EPay 配置用例首次并发运行 | `epay-min-configuration.log`：2 通过、4 在 schema 初始化阶段失败；私有 PostgreSQL 报 `out of shared memory / max_locks_per_transaction` | 复用同一编译二进制，未修改源码或断言，串行后 6/6，最终定向范围 16/16；14 输入 SHA 未变 |
| 开发机 SSH 下的完整备份第二份复制与初次 5004 导入尝试 | SSH 握手阻塞，当时无完成回执 | 5004 后经固定 GitHub runner 入口完成三阶段导入；开发机第二份完整备份复制仍 pending，原连接失败事实保留 |
| 生产旧 Core 用户页 | 实际浏览器显示“服务器返回了不兼容的用户数据” | Core 修复已随本轮发布，真实管理员同流程复验仍待完成 |

更早阶段的 vault 夹具失败、预览错误及类型检查中断继续保留在[进度记录](../progress.md)与[前轮报告](lmm-channel-routing-2026-11-18.md)。后续通过只说明对应修复后的输入，不能覆盖先前失败的事实。

## 第一批 Core 75c9d482 线上回执（历史阶段）

**本节所有包与运行摘要均对应 75c9d482 的第一次 Core 发布。** 发布包 SHA-256 为 `839d628334c8b6dd359f5087bf650741c0af0095eff61db091fac0ce29978299`；运行中 Core 可执行文件 SHA-256 为 `6c21f33fbd72f330fa146a074f472cc50dceb73dcfecaffc31dfc84b18cdefed`，与固定源码构建结果一致。该阶段回读时 Core 指向 `slots/core-a`，Core、Extensions、旧版 lmm-api 服务均 active；Core／Extensions readiness 与 Web 本地入口均返回 200。

升级前先封锁 native runtime 连接并备份，再把 21,694,958 字节备份实际恢复到专用临时数据库，确认 74 合同。备份 SHA-256 为 `f01567017926ea81ddcfcfbd47a26154b83da3c9e63f83252573a41b54c18ed1`。两次正式升级回执均为 committed，随后应用用户分组最小列授权、核验新 Core 数据库合同并切换程序。维护窗口约 34 秒；运行角色 LOGIN 已恢复，恢复验证库已删除。

最终线上合同为 76 个组件，catalog SHA-256 为 `bed3e7e3ad2810bda942473b660e4348294dab31e8fdc72e52cb5693507158db`。服务器证据保存在本轮专用升级目录；开发机 `core76-server-evidence/` 保存已取得的非秘密升级与部署回执。完整数据库备份和运行时备份保留在服务器私有位置，数据库备份已完成隔离恢复校验；开发机第二份完整备份复制因 SSH 握手阻塞尚未完成。私有备份不随公共报告上传。正式站点的认证后读写验收仍单独待完成。

## 最终发布与 native 管理反代

### 当前三组件发布（6fa1b905）

已部署应用版本为 [6fa1b9054c1a03898e082aacf28e228880c3004e](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e)。[GitHub Actions 44315659498](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498) 的 Core、Extensions、Web 全流程成功；[部署 job 134848523976](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498/job/134848523976) 于 2026-11-18 13:52:42 UTC（21:52:42 Asia/Taipei）完成，Ansible 汇总为 `ok=54 changed=14 failed=0 unreachable=0 rescued=0`。

该 job 确认 Core 正式 `check-db` 兼容检查通过，三组件 health 检查通过；实际 Core／Extensions 的 `/proc/<pid>/exe` 摘要分别与组件 manifest 匹配，served HTML 与 Web 产物匹配。上述结果构成本次 6fa1b905 的 CI 运行态验收。开发机 SSH 独立回读仍受握手阻塞，属于尚未完成的补充复核；真实管理员页面及既定写入验收等待用户完成管理员登录后继续。

### 上一批三组件发布（72392577，历史阶段）

[723925770caae52cc33a331bbfe2d758729fdc28](https://github.com/TokenNotIncluded/lmm-api-source/commit/723925770caae52cc33a331bbfe2d758729fdc28) 的 [GitHub Actions 44305784095](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095) 全流程成功；发起回执 `epay-deploy-dispatch.json` 记录 `components=core,extensions,web`、`targets=arch-shadow` 与 `version=0.0.0-config.72392577`。[部署 job 134813816947](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095/job/134813816947) 已核对实际进程可执行文件摘要与 served HTML，确认对应当时的 72392577 发布。该阶段随后完成 EPay 5004 实际导入，回执见下节。前节 75c9d482 的包和可执行文件摘要继续归属于第一次 Core 发布。

### native 管理反代

native 管理 include 已实际部署到 `native.api.lmm.best`，回执中的模板 SHA-256 为 `771b7da302c8a2c775a245eebea7bc637b2b9dcb1737ed3b1328d538cc7efde6`。部署只覆盖 native vhost；旧版主域名保持原部署。实际部署回读与随后独立公网匿名检查如下：

| 匿名请求路径 | HTTP 状态 | 证据与覆盖边界 |
| --- | --- | --- |
| `/sign-in` | 200 | `final-native-ingress.json`；登录入口可达 |
| `/core/v1/admin/user-access/1` | 401 | 部署回执与最终公网检查；合法 ID 路径进入身份校验 |
| `/core/v1/admin/user-access` | 404 | `nginx-management-deployment.json`；缺失 ID 被拒绝 |
| `/core/v1/admin/user-access/0` | 404 | 部署回执与最终公网检查；非正整数 ID 被拒绝 |
| `/core/v1/admin/users` | 401 | `nginx-management-deployment.json`；用户列表要求认证 |
| `/core/v1/payment-gateways/epay` | 403 | `final-native-ingress.json`；Core owner 配置拒绝匿名访问 |
| `/api/option/epay/config` | 401 | `final-native-ingress.json`；Go 安全投影要求认证 |

上述回执覆盖匿名入口和拒绝边界。真实管理员读取、写入与冲突恢复继续按下节单独验收。

## EPay 5004 实际导入回执

2026-11-18，固定目标 `arch-shadow` 的 stage、prepare、activate 三个独立运维阶段均成功。入口和两个已审核 helper 的固定源码为 [42c0e49be8305920051ce37021226e273c41bd35](https://github.com/TokenNotIncluded/lmm-api-source/commit/42c0e49be8305920051ce37021226e273c41bd35)；该运维提交与已部署应用版本 72392577 分别记录。阶段没有启用新支付或扩大资金权限。

| 阶段 | Actions 回执 | 实际完成结果 |
| --- | --- | --- |
| stage | [run 44312233527／job 134829045386](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44312233527/job/134829045386) | 从服务器内的旧配置建立禁用 5004 候选，完成来源与既有绑定检查；原密钥未离开服务器 |
| prepare | [run 44312648563／job 134830503978](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44312648563/job/134830503978) | 建立 Core 私有 base／初始密钥并通过正式 `check-db` 的 schema／运行权限检查；此阶段未切换主配置 |
| activate | [run 44312937277／job 134831506369](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44312937277/job/134831506369) | 切换 Core 主配置与 Go 支付配置，重启服务，Core／Extensions 的 `/health/ready` 检查通过，并再次核对三组件版本 |

Core 主配置 `/etc/lmm.conf.d/native69-20261115/core-final.toml` 的 `payments.config_file` 已指向 `/var/lib/lmm/core/config/payments.toml`。新目录由 `lmm-core` 持有，权限为 `0700`；base 与初始密钥文件为 `0600`。Core 主配置和 Go 支付配置替换时保留各自实际 uid、gid 和 mode，原配置已备份。原有 Pancake 绑定与资金权限保持原值，特别是 Go 渠道 5002 原有 `subscription_enabled=true`；新增 5004 的 Core 启用、Go 钱包及订阅开关均为 false。

开发机安全回执位于 `~/.local/state/lmm-api/channel-routing-20261118/ldc-activation-receipt.json`。最终观察为 `activation_phase=activated`、`activation_recovery_required=false`、`release_verified=true`；三阶段均为 `completed`，各自 `helper_exit_code=0`，具名 unit 的结果均为 success。候选 plan SHA-256 为 `05c36481f4b4b675270851b392aecc1dc4ea3a808dd8cfb7d1aa69a2ed50d2ab`。服务器实际执行的 helper 保持原审核摘要：

| Helper | SHA-256 |
| --- | --- |
| `stage_legacy_ldc.py` | `654e648bee5bcf8cce0a92397c652d48748ef1f5afc54b0693a28acae7ef9480` |
| `activate_legacy_ldc.py` | `181b457598de365d65eda5128479faaa9af7623a938f7cd30a0d002664697de9` |

一次性工作流与运维源码已在 [56f5f7ab](https://github.com/TokenNotIncluded/lmm-api-source/commit/56f5f7ab825d5828e5125c97b199f4facfa62a36) 从常态分支撤下，固定 Git 提交与安全回执保留；服务器支付配置、overlay 与版本密钥继续按持久配置合同保留。真实管理员读取、保存与冲突恢复仍待完成，5004 继续保持禁用；旧折扣仍需在准入新订单前独立核验。

## 后续更新所需证据

| 待完成事项 | 当前状态 | 收尾时追加的实际证据 |
| --- | --- | --- |
| 开发机 SSH 独立回读 | 6fa1b905 已有完整 CI 运行态验收；开发机连接仍受握手阻塞 | 补充独立回读的 slot／进程、文件摘要与 readiness |
| 开发机第二份完整备份 | 完整数据库／运行时备份复制未完成 | 文件字节数、复制退出码及与服务器原件一致的摘要 |
| EPay owner 配置读回 | 5004 已完成实际导入、私有 base 切换及 health 检查；真实 owner 页面尚未验收 | Core／Go 安全投影版本、5004 禁用状态及最低额的实际页面回执；报告不包含密钥 |
| 真实管理员浏览器读取 | 官方扩展桥已连接；后台标签 1 稳定在正常密码登录页，无 alert，等待用户完成管理员登录 | 登录后继续渠道、用户列表／详情与 EPay 配置的真实页面读取 |
| 管理员保存与冲突恢复 | 本次浏览器未执行生产渠道、用户权限、Pancake 或 EPay 配置写入；预定 EPay 返回 URL 测试尚未执行 | 登录后继续既定 EPay 返回 URL 保存／读回／恢复测试；本轮用户分组只读，不保存 |
| 渠道超时修复页面读回 | 6fa1b905 源码、定向验证、三组件发布及 CI 运行态验收均完成；等待管理员登录后继续页面读回 | 实际运行下限、新目标默认值及发布错误提示的真实管理员页面结果 |
| 正式启用与真实交易 | 本轮未执行 | 另行明确的启用范围、实际交易与核账结果 |

### 真实浏览器交接

官方 Playwright 扩展桥已连接，真实后台页面操作已开始。为继续验收创建了独立后台标签 1，标签 0 留在 wallet；标签 1 的 `document.visibilityState=hidden`。在标签 1 按既有“恢复浏览器会话”→“重新登录”流程操作后，页面稳定在 [管理员登录页](https://native.api.lmm.best/sign-in)，正常密码登录表单存在、alerts 为空。当前等待用户在该后台标签以管理员账号完成登录。

最初访问 `/channels` 时发生认证会话中断，沿既有 UI 恢复后曾进入 dashboard／overview 及 wallet。这些观察不构成渠道、用户或支付配置验收通过，也不足以判定渠道权限失败；会话中断起因尚未确定。本次未提交登录表单，也未直接读取或改写认证存储；会话恢复使用应用既有按钮。未执行生产渠道、用户权限、Pancake 或 EPay 配置写入，预定 EPay 返回 URL 保存／读回／恢复测试尚未执行。管理员业务页面验收仍待继续；本轮用户分组只读，不保存，以免撤销会话与 API key。

最终只读观察回执为开发机 `~/.local/state/lmm-api/browser-mcp-20261118-owtg7u1i/receipts/native-acceptance-final-handoff-state-1.json`，记录正常登录页、hidden、alerts 为空、密码表单存在、管理员组未出现，不包含表单输入值。

连接前的历史准备证据仍保留在 `~/.local/state/lmm-api/browser-mcp-20261118-owtg7u1i/preparation.json`：当时任务私有目录已安装 `@playwright/mcp@0.0.85`，完成 MCP 握手与 25 项工具 schema 读取，桥为 prepared、`armed=false`、页面工具调用为 0。这些值仅描述连接前状态，不能代替上述当前观察。

SSH 阻塞期间保留服务器备份、隔离恢复回执与开发机已取得的非秘密证据。5004 已通过 GitHub runner 完成导入；渠道超时修复 6fa1b905 也已完成三组件发布及 CI 运行态验收。开发机第二份完整备份复制尚未完成；研发与部署完成不代表正式上线业务验收完成。用户在独立后台标签完成管理员登录后，继续页面读回及既定验收。

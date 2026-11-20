# LMM provider 迭代报告（2026-11-20）

Store 已进入主线；Pi 补修仍在 draft PR；Core [draft PR #11](https://github.com/TokenNotIncluded/lmm-api-source/pull/11) 包含实现 `8f523bef` 与测试增强 `157b4f05`，尚未合并主线。
Core 普通唯一用例最终 22 项通过；首次 Pi 运行时失败保留，schema capture 1 项与 Core77 对照 1 项分别另计。
下列 `out/` 路径相对于开发工作区 `lmm-api/`，用于定位原始日志与收据。
在线汇总同步到 [Google 文档](https://docs.google.com/document/d/1uwyaZy0lWgquJAG6CoPn5zF_IMoUtmcYvhjzIDktH_c/edit?usp=drivesdk)。

## 结果与版本归属

| 范围 | 本轮版本 | 当前结果 | 验证边界 |
| --- | --- | --- | --- |
| Store / Toolmarket | [主线 3d1c09d5](https://github.com/TokenNotIncluded/lmm-api-source/commit/3d1c09d5e0424b44603686f7419e76f790b2bb5c) | 5 文件已提交；主线 MerchantHome 真实 PG race 通过 | Toolmarket 两项复用 b5ee 原证据 |
| Pi npm 12 兼容 | [b78228a1](https://github.com/TokenNotIncluded/pi-lmm-provider/commit/b78228a17d4b7b883bec9b9b1ca2915674bf8dc6) | 167 项、类型、打包与真实宿主加载通过 | Pi 0.86.1 单行；其余 5 行未重跑 |
| Core 五个公共客户端 | 实现 [8f523bef](https://github.com/TokenNotIncluded/lmm-api-source/commit/8f523befd7be5491c6e62183049802acedfcaf2d)；测试 [157b4f05](https://github.com/TokenNotIncluded/lmm-api-source/commit/157b4f0598fd865146d1ce1ea634f0d1c501213c) | 普通唯一用例最终 22 项通过，原始失败保留 | main 仍为 `3d1c09d5`；PR #11 保持 draft |

最终 Pi 精确复验只增强成功 SSE 末事件检查；生产行为、超时和原 49 credits 结算断言均未改变。
[Pi PR 16](https://github.com/TokenNotIncluded/pi-lmm-provider/pull/16) 回读时仍为 open / draft，head 已为
[9598fc94](https://github.com/TokenNotIncluded/pi-lmm-provider/commit/9598fc94933f5d04bf9382c827b7e228fa6c8a38)，另一维护链调整宿主依赖安装并保留 npm 12 补修。
本报告的 167 项与宿主加载结果只绑定 `b78228a1`，不能转移给新 head。
并行候选 `review/core-public-providers-20261120@67880af3` 及历史报告独立保留，不能继承本轮组件 78 的验证资格。

## Store：主线实测与历史证据

主线提交区分 Toolmarket 安装者与运行时角色，覆盖丢失 COMMIT 确认后的恢复；
独立 Store PG 模块补上已有 `purego` 依赖和校验值，模块说明同步实际 Core bridge 与宿主。

| 检查 | 结果 | 结果所属版本 |
| --- | --- | --- |
| MerchantHome，PG 18.7，Go race | 1/1 通过，0 skip | 主线工作树实测，随后映射到 `3d1c09d5` |
| Toolmarket 两项真实 PG 用例 | 复用原有通过证据 | [b5ee7158](https://github.com/TokenNotIncluded/lmm-api-source/commit/b5ee7158157f48b501da526bca96b39c63b50420)；已核对运行源码输入一致 |
| 独立模块 readonly vet、格式与 docs 检查 | 通过 | 本轮主线改动 |

复用的是 `TestPostgresStorePersistenceConcurrentRollbackAndRestart` 与
`TestPostgresMarketAIReviewLostCommitACKRestoresPreviousLiveVersion`，原环境 Go 1.27.2 / PG 18.6。
这两项没有在 `3d1c09d5` 上重跑，保留原提交和输入归属。

## Pi：npm 12 单元素 manifest 数组

官方 npm registry 的精确宿主查询返回单元素数组，旧 `select-host.mjs` 按对象读取包名而失败。
补修仅展开该数组，仍验证唯一包、名称、版本、依赖及真实宿主 import。
环境为 Node.js 26.13.1 / npm 12.2.0 / Pi 0.86.1；167/167 测试通过，0 fail、0 skip，类型检查通过。
`pack:check` 检查 28 个打包文件；真实 Pi 加载注册 `lmm-prices`、`lmm-revoke`、
`lmm-diagnostics`、`lmm-cache`、`lmm-currency` 五个命令。本轮未发布 npm 包。

## Core：有限授权与固定升级目标

五个客户端默认关闭，采用原生授权码及 PKCE S256，在 `identity.oauth2.provider_max_credits` 分别配置有限正数上限：

| 客户端 | Client ID |
| --- | --- |
| DSH | `lmm-dsh` |
| OpenCode | `lmm-opencode` |
| CodeWhale | `lmm-codewhale` |
| VS Code | `lmm-vscode` |
| Zed | `lmm-zed` |

浏览器确认本次授权族预算，刷新共享累计消费与预留；普通客户端无需 managed 安装字段。
配置要求实际模型服务；Pi 的 `pi_max_credits`、CC Switch / Magpie 的 managed 合同保持独立。
停用后拒绝新授权和刷新，仍允许撤销已有令牌；配置及按账户策略验证 MFA 见[客户端文档](../oauth-client-metadata.md)。

`provider-clients` 为组件 78，固定 `core-provider-clients-v1` 从精确 77 升级；普通启动只校验契约。
隔离 PG 捕获的目标已固定在[目标文件](../../services/core/src/core_schema/upgrade/provider_clients_target.json)，旧 managed70 DDL 与 pins 保留：

| 组件 78 指纹 | SHA256 |
| --- | --- |
| manifest | `b9d0a472bddc826e0f6bfa0e725e409e67cbaabad6b0ad80cd512de24da74c2b` |
| catalog | `dbc1df98f792d1484700e36005e900ec97a54a21a56d752bab0863bd84ab7177` |

## 财务断言与当前回归结果

新增 HTTP 生命周期用例验证五个客户端的授权、刷新、错误客户端拒绝及撤销，五次合成上游调用各结算 49 credits。
账单按 client ID 核对归属与付款账户，`key_id` 均为空；最终余额 755、预留 0，API key 数量不变。
第二项在同一授权族上限 70 下，刷新前后两笔并发预留 50 仅一笔成功；结算 40 后，再预留 31 被拒绝。
停用客户端仍可撤销，最终余额 960、预留 0、上游调用 0。HTTP 首次私有字段编译错误改用 `client_id()` 后，2/2 通过。

| 验证范围 | 结果 | 唯一用例计数 |
| --- | --- | --- |
| provider78 / managed70 真实 PG | 通过，0 skip | 各 3 项，共 6 项 |
| upgrade registry | 通过 | 5 项 |
| protocol / config / consent CSP / startup gateway | 通过 | 4 项 |
| 新 provider HTTP 财务 | 通过 | 2 项 |
| Pi 浏览器 / managed consent 与 discovery / Magpie | 通过 | 1 + 2 + 1 项 |
| 旧 Pi 实际 Core TCP chat / Responses 流式与 native control | 157b4f05 精确复验通过，首次失败保留 | 1 项，计入最终 22 项普通唯一用例 |
| provider78 结构 capture | 通过 | 显式 1 项，另计 |

用例 `actual_core_tcp_pi_chat_responses_json_stream_and_native_control` 首次失败时 chat 流预留 140、结算 0；
已观察到 terminal，仍无结算，并有 2 秒 charging lock timeout 证据；预期结算 49 的断言未削弱。
严格 Core77@`3d1c09d5` 单次对照 1/1 通过：测试本体 11.93 秒，含编译的进程总耗时 294.82 秒；单次对照不足以独立确定因果。
组件 78 早期诊断编译失败，另两次 setup 在 cargo 前退出，均未执行 case，不能计作第二次结算失败。
恢复正确构建后，157b4f05 精确复验 1/1 通过：测试本体 5.10 秒、含编译进程 287.131 秒。该通过不抹除原失败，根因未唯一确认；六次离散采样未见请求锁阻塞/锁超时，但包含 WALWrite / WalSync 等待。

初次 capture 的 `EmptyHost` 在 SQL 前退出；group 编码及 AF_UNIX 路径过长两处夹具问题均已修复并通过复验。
SQL/pins 未因这些夹具修复改变，原始失败与成功复验均保留；HTTP 私有字段编译失败也保留在首轮收据。

## 证据索引

| 证据 | 工作区内路径 | 范围 |
| --- | --- | --- |
| Store MerchantHome | `out/lmm-market-main65-local/merchant-home-homefs/receipt.json` | 主线 race 1/1；同目录 `merchant-home.stdout` |
| Store vet / docs | `out/lmm-market-main65-local/nested-pgtest-vet.stdout`、同目录 `docs-only.stdout` | 同目录 `receipt.json` 的 vet/docs/gofmt 步骤各 exit 0；整体夹具首轮 FAIL 保留 |
| Toolmarket 原证据 | `out/lmm-market-pg-b5ee-ci/terminal-review-44719155709.json` | 原包为同目录 `artifact/market-postgres-44719155709.zip` |
| Pi 原失败 / 补修 | `out/lmm-pi-104-cohort-metadata-20261120/local-1eb82ee8-node26/`、同层 `local-npm12-fix-node26/receipt.json` | b78228a1 单行，02–06 日志 |
| Core schema 汇总 | `out/lmm-provider-clients-local/provider-schema-verification.json` | 6 PG + 5 registry，capture 1 另列，含原失败与复验哈希 |
| Core 首轮 | `out/lmm-provider-clients-local/core-root-tests-receipt.json` | 4 项通过及 HTTP target 首次编译失败 |
| Core 修正后复验 | `out/lmm-provider-clients-local/core-root-recheck-receipt.json` | 新 HTTP 2 项、Pi 浏览器及旧 Pi 流式结果 |
| Core 其余回归 | `out/lmm-provider-clients-local/core-root-remaining-receipt.json` | managed / Magpie |
| Core78 精确诊断 | `out/lmm-provider-clients-local/core-pi78-diagnostic-receipt.json` | 首次编译失败，15.4 秒，case 未执行；后续成功复验见下一行 |
| Core78 修正构建后复验 | `out/lmm-provider-clients-local/core-pi78-final-receipt.json` | 157b4f05，1/1 通过；同目录 `core-pi78-final.log` 与 `core-pi78-final-lock-trace.jsonl` 保留 |
| Core77 单次对照 | `out/lmm-pi-core77-control-20261120-115438/control-case-receipt.json` | 1/1，测试本体 11.93 秒，仅作一次对照 |

完整路径、SHA256、状态与提交归属见[机器索引](lmm-provider-validation-2026-11-20.json)。复验未唯一确认原失败根因；PR #11 保持 draft，Core 候选未合并 main；本轮未部署或升级生产数据库。

## 清理与输入核对

最终 Core 输入与此前通过用例的输入仅相差 Pi 流式测试文件；生产源码没有变化。
本轮两个私有 PostgreSQL 实例、数据库数据和临时凭据均已清理，原失败、成功复验、日志与收据保留。
最终清理收据为 `out/lmm-provider-clients-local/final-cleanup-receipt.json`；Core77 对照清理收据单独保留。

# LMM API：Key quota 验证与 Core77 社区连接历史导入

日期：2026-11-19（Asia/Taipei）。性质：开发迭代与定向自测。**self-tested; independent acceptance not run**。

## 结论与范围

本轮修复当前原生 Key quota 测试的结构合同发现方式，增加 Core76→77 的显式离线升级和连接历史导入目标，并修复社区单独启用时遗漏的设置表读取权限。

Key quota 在 Core76 上的 8 项业务测试及 1 项辅助测试全部通过；同一条导入→计费业务在 Core77 上另做了一次回归。社区的注册表、manifest、真实数据库升级和历史导入运行链共 8 个不同检查分阶段通过。结果按各阶段的源码和二进制收据分别保存，没有把重复运行计为新用例。

上述结果覆盖当前原生额度路径和连接历史分析。冻结 Core69 身份导入的完整生产迁移资格、真实历史数据对账、完整候选后端 CI 和独立业务验收仍未完成。本轮未发布新版本、部署服务、切流或调用付费供应商。

## 一、修复 Key quota 的测试前置条件

先前夹具初始化当前 Core76 数据库，却通过冻结 Core69 身份导入器发现目标合同，因而在额度业务开始前返回 `component_rust_source_fingerprint_mismatch`。这项失败反映的是夹具使用了不匹配的结构说明，不能据此判定额度业务通过或失败。

现在夹具先调用 `core_schema::check` 校验实际数据库，再采用同一编译产物的 `schema_manifest` 和运行目录指纹。生产额度写入器仍在事务中重新检查实际目录、组件及身份条件。原有 stdin 授权脚本补齐同目录 owner 检查文件的展开，并显式传入 `owner_role`。

已提交：[b4414aa5](https://github.com/TokenNotIncluded/lmm-api-source/commit/b4414aa5849f2c1bfc4dd8678c15af1d6d52df48)。冻结 Core69 导入器及其审核指纹没有修改；本轮修复不能替代 Core69→当前版本的历史导入审核。

## 二、提供 Core77 连接历史导入与升级

旧连接历史导入只接受 Core55/56 的精确合同。新实现增加第 77 个组件 `connection-history-import`，为当前数据库提供对应入口；冻结的 55/56 SQL 和已有版本的目标指纹继续保留。

升级程序 `core-connection-history-import-v1` 只接受精确 Core76 前驱，通过既有离线升级框架进入 Core77。请求绑定数据库身份、owner/installer、manifest 与 catalog；同一请求可返回已提交状态，修改后的请求被拒绝。新增收据保留请求、数据、权限与目录摘要，运行角色不能读取或修改升级收据。服务启动不会自动执行升级。

Core77 的目录指纹来自真实 Core76 执行 `init-db`、独占 owner 应用第 77 组件 DDL 后的 PostgreSQL 18.6 实测。随后 Rust 升级测试验证了实际升级、数据与 ACL 保持、精确重放及冲突拒绝。计划器继续拒绝 Core76、未审核的 Core78 和混合指纹。

这里导入的是连接历史分析资料，包括保留的 API Key、OAuth、申请和审核历史。它不创建可用凭据、资金余额或运行授权，也不代表全部旧版社区数据已完成迁移。使用说明见 [连接历史导入说明](../../scripts/legacy_connection_import.md)。

## 三、补齐社区读取权限及撤销

真实数据库测试发现：仅启用 community 的运行角色没有 `core_console` schema 的使用权，也没有设置表 `key/value` 列的读取权；`CommunityRead::new` 读取显示汇率时返回 SQLSTATE `42501`。

修复同时覆盖三个位置：

- 授权脚本只增加 schema 使用权与 `core_console.options(key,value)` 的读取权。
- 运行能力检查要求这三项权限齐全，使缺权在启动前被发现。
- 撤销脚本显式撤销列级读取权。PostgreSQL 的表级 REVOKE 不会清除单独授予的列级权限。

集成测试保留原历史快照业务断言，并检查正常启动、逐项缺权拒绝、不能读取 revision、不能写设置表，以及完整撤权后的权限清理。权限状态由 owner 以目标角色参数查询，避免用已撤权的角色解析表名。

代码提交：[2991afc7](https://github.com/TokenNotIncluded/lmm-api-source/commit/2991afc7eac02f812abed17dbd519f72cf96f6ec)。

## 实际验证结果

| 验证范围 | 结果 | 证据与解释 |
|---|---|---|
| Core76 quota | 9/9，通过，43.140 秒 | 8 项真实 PostgreSQL 业务 + 1 项 hex 辅助用例 |
| Core77 quota | 1/1，通过，5.425 秒 | 精确复验 `offline_writer_plan_and_charging_engine_admission`；与上行同一业务用例，不增加不同用例数 |
| Core77 注册表 | 5/5，通过 | 程序集合、阶段入口、拒绝策略、授权收据与故障默认值 |
| manifest CLI | 1/1，通过 | 无配置时输出精确机器 JSON |
| Core76→77 升级 | 1/1，通过，4.072 秒 | 真实 PostgreSQL；数据、ACL、收据及重放 |
| 社区历史导入运行链 | 最终 1/1，通过，5.726 秒 | 最终权限修复产物；历史导入、读取和权限拒绝/撤销 |
| Python 计划器 | 5/5，通过 | 精确版本与目录合同 |
| 修改范围检查 | 通过 | Rust 格式、差异检查及 docs-only 检查 |

额度测试覆盖实际 Key 创建与摘要绑定、离线额度导入后计费准入、跨 Key/账户拒绝、有限/无限额度切换不清空已用额度、并发与精确重放、失败回滚所有表及序列，以及未知结果按原迁移 ID 查询。

构建记录分别为：Core76 quota 250.951 秒；初始 Core77 联合构建 387.771 秒；社区授权修复定向构建 139.438 秒；最终仅测试观测连接调整的构建 9.686 秒。最终构建记录的 16 个输入文件无漂移。未改变的 schema/升级/注册表结果沿用原收据；quota 的 Core77 回归在 community 关闭条件下执行，后续仅社区授权变更不涉及这条路径。没有将这些结果描述为完整候选后端测试。

### 失败记录与处理

1. 社区首轮执行 8 项，7 通过、历史导入运行链 1 失败。实库诊断确认缺少 `core_console` 权限。原失败收据及 SQLSTATE 保留，修复上述授权、能力检查和撤销逻辑。
2. 第一次修复复验已通过构造器、三项缺权拒绝及原快照撤权检查，停在新增的完整撤权断言：被撤权角色无法按名字解析设置表。仅把权限观测查询改为 owner 查询目标角色，保留实际产品权限与业务断言；随后同一条集成测试通过。

3. 第二次复验已通过数据库和权限链路，Go 子进程退出 0，但接口用例因缺少独立历史数据库地址而 SKIP，原有 PASS 断言正确拒绝。另建独占的 Go PostgreSQL 实例，配置独立 acquisition 数据库及受限连接，并从 Go 子进程移除 Core 数据库环境后，保持源码和产物不变，重新执行原集成用例。

三次失败夹具均在确认身份、隔离连接后清理；原失败结果不改写成成功。最终成功夹具也已清理，临时数据库凭据已移除。Quota 测试容器执行前后均为空。

## 结构合同

| 项目 | SHA-256 |
|---|---|
| Core76 manifest | `2c14283c2d35580f3fe314b7de6a88e3aec3524e74157310bf5022415367ff8e` |
| Core76 catalog | `bed3e7e3ad2810bda942473b660e4348294dab31e8fdc72e52cb5693507158db` |
| 第 77 组件 SQL | `1d93a2d54b797a2d3d9ac6e1b88c53f96757f6955ee8c320dfbfaa5917fd2d66` |
| Core77 manifest | `45ff3f3ddb387dc2c587effd739bfb4cc152b50864511ab10e8d48389e6a3abe` |
| Core77 catalog | `151c66efa9e5d0af276e1b69a541b432c46d6c1c22d6e7838158595d2e110905` |

部署 Core77 到仍为 Core76 的实例前，需要按 owner 离线流程执行精确升级，并重新配置所需运行权限。本轮仅在独占测试数据库验证这一过程，没有操作生产数据库。

## 证据入口

[结构化证据](lmm-migration-closure-2026-11-19.evidence.json)包含各阶段源码与测试记录、Core77 各构建的实际产物摘要，以及失败与清理收据摘要。Core76 原始收据记录了可执行路径及 quota 文件摘要，没有记录 ELF 摘要；报告不从后续覆盖的产物反推该值。开发机父项目下保留原始收据和日志：

- `out/lmm-migration-closure-20261119/quota-build.json`
- `out/lmm-migration-closure-20261119/quota-current76/result.json`
- `out/lmm-migration-closure-20261119/community-build.json`
- `out/lmm-migration-closure-20261119/quota-current77/result.json`
- `out/community-history77-20261119/target-capture.json`
- `out/community-history77-20261119/selftest-run1/`
- `out/lmm-migration-closure-20261119/community-grants-build.json`
- `out/community-history77-20261119/history-retry1/`
- `out/community-history77-20261119/history-retry2/`
- `out/lmm-migration-closure-20261119/community-grants-build2.json`
- `out/community-history77-20261119/history-retry3/result.json`

临时访问凭据不进入报告、GitHub 或共享文件。原有工作区的未提交变更保留，本轮复用现有主线副本和构建缓存。

## 后续范围

1. 为冻结 Core69 身份/Key 历史导入建立完整、可审核的当前版本路径，并完成真实历史数据与额度对账。当前原生夹具通过不能替代该资格。
2. 对明确的候选提交完成完整后端 CI 与独立业务验收。本轮遵守日常开发约定，使用定向自测及 `[skip ci]` 提交，没有触发或等待远程完整 CI。
3. 完成上述门槛后，再按既有发布与部署流程推进。已有 v0.1.21 的发布状态与上一轮 CI 结果见[前轮报告](lmm-ci-followup-2026-11-19.md)。

## 结论自检

贡献限定为当前原生额度验证、Core77 连接历史入口及社区只读授权；表达区分历史导入、当前路径与生产资格；每项测试结论对应原收据；覆盖范围不合并重复用例或既往发布统计；实现保留冻结历史合同和显式 owner 升级边界。本文不据定向自测推断全站或生产可用。

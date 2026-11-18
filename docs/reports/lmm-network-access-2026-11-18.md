# LMM API 网络访问控制迭代记录

日期：2026-11-18（UTC+8）

## 本轮结果

已提交并推送可重复运行的网络访问控制自测入口，源码提交为 [e1b461a7](https://github.com/TokenNotIncluded/lmm-api-source/commit/e1b461a7b710ae10e56e751b67a61db57512885a)。GitHub 已读回该提交，签名验证通过。本轮修改两个文件，新增 58 行、删除 7 行。

Core 运行测试已在开发机的独立 PostgreSQL 实例启动。断线前仍在编译生产库，随后 ssmcp 返回零个在线客户端，未能取得本轮退出码和测试汇总。因此本轮 Core 测试结果记为 **UNKNOWN（待回读）**；现有原生 HTTP、用户白名单与升级回执验证仍待完成。

## 已交付的改动

| 文件 | 改动 |
| --- | --- |
| `scripts/test-network-access.sh` | 创建私有 Unix socket PostgreSQL 实例，集中运行现有网络访问控制用例；默认单任务构建、串行测试；结束时停止本轮数据库，通过时清理临时目录，失败时保留现场。 |
| `docs/network-access.md` | 接入统一自测命令，说明环境准备、测试范围、低内存构建参数及失败现场位置。 |

脚本主动清除继承的 PostgreSQL 连接变量，并设置本轮自己的 `DATABASE_URL`。测试使用 SQLx 创建的独立数据库和合成账户。运行角色由测试夹具创建，权限验证仍走真实 PostgreSQL。

实现复用了开发机现有集成工作区，保留原开发目录及并行渠道、定价和前端改动。生产访问控制、资金逻辑和部署配置没有在本轮改动。

## 脚本选择的已有用例

下列是本轮选择的验证范围，执行结果尚待回读：

| 范围 | 关键行为 |
| --- | --- |
| 平台地域入口 | MMDB 地域判定、来源地址与伪造转发头、规则更新、健康检查路径。 |
| 用户白名单 | IPv4、IPv6、CIDR 与映射地址规范化；账户隔离；保留其他用户设置；拒绝保存会锁住当前来源的名单。 |
| 登录和 Cookie 恢复 | 被拒绝来源不能签发新凭据、刷新 Cookie 或改变会话快照；合法来源继续恢复原会话。 |
| 用户与 API Key 规则 | 两层来源范围取交集，余额等既有权限继续生效。 |
| 管理设置与审计 | 受限运行角色的保存与读回；版本比较冲突；审计失败时整笔回滚。 |
| 第 75 → 76 组件升级 | 精确前置合同、数据与权限保留、升级回执及重复请求重放。 |

## 实际检查记录

| 检查 | 结果 |
| --- | --- |
| `bash -n scripts/test-network-access.sh` | 退出码 0。 |
| `python3 -B scripts/check-docs-brand.py --docs-only` | 首笔提交的文档修改检查通过，退出码 0。 |
| 修改范围的 `git diff --check` | 退出码 0。 |
| Git 提交与推送 | 成功，`e1b461a7` 已由 GitHub 读回。 |
| 真实隔离库 Core 用例 | 已启动；断线前仍编译，退出码与通过数量未知。 |
| 独立业务验收、生产升级与部署 | 未执行。 |

后续这份状态记录通过 GitHub 连接补交；开发机离线期间没有重跑仓库文档检查器。

测试在 17:05（UTC+8）启动。生产源码基准为 `be27b6133ac6d0ea5ae684631be6660da35dbcbc`，新测试脚本随后纳入上述提交；首笔提交只改变脚本和文档。现场工具链为 Rust/Cargo 1.100.0、PostgreSQL 18.6。构建使用一个任务并关闭调试信息与增量编译，开始时等待开发机已有的全局 Rust 编译锁，随后进入 Core 编译。

## 复现入口与接续位置

在源码仓库根目录执行：

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  mise exec -- bash scripts/test-network-access.sh
```

脚本使用锁定且已缓存的依赖，不需要提供业务数据库地址。

本轮收据和完整日志保存在开发机父仓库的以下相对位置：

- `out/lmm-network-access-20261118/network-tests-1.json`
- `out/lmm-network-access-20261118/network-tests-1.log`

连接恢复后，先读取这两个文件与既有进程状态，确认本次运行的退出码、用例结果和临时数据库停机状态。仍在运行的任务继续沿用；若发现实际失败，按失败项修复并定向复测，再更新 [架构进度](../progress.md)。当前临时数据库的最终清理状态也待回读。

## 证据与结论边界

已确认的交付是隔离自测入口、运行说明、静态检查及已推送源码。Core 运行结果保持未知，既有 PostgreSQL SQL 检查和前端测试记录继续保留其原版本归属。本轮未新增任何后端运行测试通过项，也未宣称网络控制已经上线。

代码与说明：[e1b461a7 提交](https://github.com/TokenNotIncluded/lmm-api-source/commit/e1b461a7b710ae10e56e751b67a61db57512885a)。当前项目状态以 [docs/progress.md](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md) 为准。

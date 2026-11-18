# LMM API 网络访问控制迭代记录

日期：2026-11-18（UTC+8）

## 本轮结果

网络访问控制专项 **19 项通过、0 项失败，退出码 0**。验证固定在源码 [ef8d0130](https://github.com/TokenNotIncluded/lmm-api-source/commit/ef8d01304ecff6b7005c05222e20e37d5a20384c)，于 17:40:11 开始、17:46:51 完成，总耗时约 400 秒，其中 Rust 编译 6 分 16 秒。

本轮补齐了此前因编译中断而缺失的原生 HTTP、用户白名单及 75→76 升级回执运行验证，同时交付可重复运行的隔离测试脚本，并修复实际运行暴露的一处测试数据库依赖缺失。

## 已交付的改动

| 文件 | 改动 |
| --- | --- |
| `scripts/test-network-access.sh` | 新建私有 PostgreSQL 集群，集中运行现有网络访问专项；停止数据库并按结果清理或保留现场。 |
| `docs/network-access.md` | 统一自测命令、范围、环境准备和低内存构建参数。 |
| `services/core/src/identity/network_policy.rs` | 为 Key 与用户白名单交集用例补齐 abuse schema，修改限于测试模块。 |
| `docs/progress.md` | 将该项后端验证从待回读更新为实际通过，并保留中断及失败历史。 |
| 本报告 | 记录原因、固定版本、测试结果与复现位置。 |

脚本与运行说明已在 [e1b461a7](https://github.com/TokenNotIncluded/lmm-api-source/commit/e1b461a7b710ae10e56e751b67a61db57512885a) 提交，夹具修复已在 [ef8d0130](https://github.com/TokenNotIncluded/lmm-api-source/commit/ef8d01304ecff6b7005c05222e20e37d5a20384c) 提交；两笔均已推送并经 GitHub 回读，签名验证通过。[9eec78d9](https://github.com/TokenNotIncluded/lmm-api-source/commit/9eec78d9b9b78cab07dddb868da1a69e3f0e39a5) 保留了连接中断时的阶段记录，本报告以恢复后的结果更新。

## 实际验证结果

| 测试目标 | 通过 | 失败 | 验证范围 |
| --- | ---: | ---: | --- |
| Core lib 中选定的网络相关用例 | 14 | 0 | 配置校验、MMDB 与可信来源、地址规范化、用户/Key 白名单交集、登录与会话保护、75→76 数据及权限保留、升级回执重放。 |
| `network_access_options` | 2 | 0 | 真实受限运行角色的配置保存与读回、直接表写拒绝、比较保存冲突、审计失败时原子回滚。 |
| `user_network_access` | 3 | 0 | HTTP 设置持久化与账户隔离、凭据和 Cookie 恢复限制、用户名单与 Key 名单交集。 |
| **合计** | **19** | **0** | 网络访问控制专项开发自测。 |

最终日志摘要：

```text
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1818 filtered out
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
服务器进程已经关闭
```

Bash 语法检查、修改文件的 Rust 格式检查、文档检查器 `python3 -B scripts/check-docs-brand.py --docs-only` 与本轮差异检查均退出 0。

## 执行过程与失败保留

| 运行 | 结果 | 处理 |
| --- | --- | --- |
| `network-tests-1`，17:05 启动 | 连接重启时仍在编译；恢复后原进程已不存在，没有最终退出码。 | 标记 interrupted，不计入通过数，保留原收据与日志。 |
| `network-tests-2`，17:27–17:35 | lib 13 通过、1 失败，退出码 101；后续接口组未执行。 | PostgreSQL 日志定位缺少 abuse 表，修复实际夹具依赖。 |
| `network-tests-3`，17:40–17:46 | 19 通过、0 失败，退出码 0。 | 记录最终收据；数据库正常停机，成功临时目录已清理。 |

第二次失败时保留的数据库已经停机，其 PostgreSQL 日志归档后清理临时目录。最终回读没有残留的本轮 `/tmp/lmmnetwork.*` 目录。首轮中断的原始退出状态保持未知，没有补写为成功。

## 修复原因

首次完整运行停在 `user_ip_allowlist_intersects_key_policy_without_http_context`：创建 API Key 时返回 `Storage`，隔离 PostgreSQL 日志明确记录 `relation "core_abuse.settings" does not exist`。

该用例仅调用 `IdentityStore::init_database()`，安装身份组件；`issue_key()` 还会读取滥用防护的 settings 和 user_marks 表。本轮沿用仓库已有测试方式，在该用例中安装真实 `crate::abuse::SCHEMA`。改动位于 `#[cfg(test)]` 模块，共新增 6 行，保留原白名单断言。两个 HTTP 集成测试已使用完整 Core 初始化，不需要重复安装依赖。

## 可重复运行的入口

在源码仓库根目录执行：

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  mise exec -- bash scripts/test-network-access.sh
```

脚本清除继承的 PostgreSQL 连接变量，新建私有 Unix socket PostgreSQL 集群，使用 SQLx 创建的隔离数据库和合成账户；默认单任务构建、串行执行用例。结束时停止本轮数据库；通过时清理临时目录，失败时保留现场。依赖使用锁定版本和本机缓存。

本轮现场工具链为 Rust/Cargo 1.100.0、PostgreSQL 18.6。长任务由开发机临时用户服务承载，避免连接重启再次中断编译。

## 证据位置

开发机父仓库的相对路径：

- `out/lmm-network-access-20261118/network-tests-1.json` 与 `.log`：首次中断。
- `out/lmm-network-access-20261118/network-tests-2.json` 与 `.log`：首次取得的真实失败。
- `out/lmm-network-access-20261118/network-tests-2-postgres.log`：缺表错误与数据库停机记录。
- `out/lmm-network-access-20261118/network-tests-3.json` 与 `.log`：修复后的最终通过结果。

最终完整日志 SHA256：`189e7a7518cb9318f1064f0429aaa33930e184d351418246ad300eda78bc4528`。

## 结论边界

结果属于开发自测，范围为网络访问控制专项，不代表全量 Core 测试或独立业务验收。既有前端测试、原始 SQL 测量与本轮 Rust 结果分别保留版本归属。本轮未执行生产数据库升级、部署或限制策略启用。

当前项目状态以 [docs/progress.md](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md) 为准。

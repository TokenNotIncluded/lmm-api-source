# LMM API 数据库升级回归修复

日期：2026-11-20（Asia/Taipei）  
状态：**self-tested; independent acceptance not run**

本轮修复了两处数据库升级测试问题：Go 钱包升级夹具继承 PostgreSQL 连接环境，以及 Rust 历史升级夹具在数据库连接进程退出前请求独占升级。Go 的四个顶层测试及三个子场景通过竞态检测；Rust 的八条历史升级路径分阶段通过真实隔离数据库自测。

## 修改与原因

### 钱包升级测试的连接环境隔离

既有 [CI #141](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44527029295) 的前序任务设置了 `PGHOST`、`PGPORT`、`PGUSER`、`PGPASSWORD`、`PGDATABASE`、`PGSSLMODE`。钱包升级测试自行启动临时 Unix socket 数据库，但继承了这些变量，触发安装器对隐式连接环境的拒绝。

[修复提交 e312c77a](https://github.com/TokenNotIncluded/lmm-api-source/commit/e312c77a9e5d128d7aa5e09146e66d321cc2bec2) 在本包 `TestMain` 创建临时数据库前清理继承的 `PG*` 变量。生产连接校验继续执行原有规则。开发机用相同变量名称和合成值复现了安装拒绝；修复后，在同一环境条件下运行整个包，退出码为 0。

```sh
GOWORK=off GOMAXPROCS=2 go test -p 1 -race ./internal/walletupgrade -count=1 -v
```

### 历史升级测试的连接关闭顺序

[修复提交 827d0e08](https://github.com/TokenNotIncluded/lmm-api-source/commit/827d0e087650757fa11772eb5cefeface7ecab26) 在关闭两个测试连接池前记录本夹具的数据库连接进程 ID，随后通过维护数据库观察这些进程退出，再调用独占升级入口。原有检查连接复用同一等待函数。升级入口的独占要求及拒绝行为继续由原有实现执行。

这次改动位于公共测试夹具 `services/core/tests/common/suffix_upgrade.rs`。测试调用固定历史 Core 执行器，核对历史数据、账本、权限、序列状态、原升级回执与重复执行结果。

## 本机自测

测试使用开发机上的独立临时 PostgreSQL 数据库及合成身份。Rust 环境为 `postgres (PostgreSQL) 18.6`；Go 环境为 PostgreSQL 18.6。每条 Rust 路径运行公共升级业务用例一次，以下八条为不同路径的通过结果。

| 历史升级路径 | 测试目标 | 结果 | 用时 |
| --- | --- | --- | ---: |
| 账号申诉 | `account_appeals_upgrade` | 1/1 通过 | 10.187 秒 |
| 交易与显示汇率 | `commerce_display_rate_upgrade` | 1/1 通过 | 19.805 秒 |
| 渠道账务 | `channel_accounting_upgrade` | 1/1 通过 | 8.421 秒 |
| 渠道观测 | `channel_observations_upgrade` | 1/1 通过 | 7.306 秒 |
| 渠道投影 | `channel_projection_upgrade` | 1/1 通过 | 7.660 秒 |
| 导航配置 | `navigation_options_upgrade` | 1/1 通过 | 7.712 秒 |
| Pancake 结账语言 | `subscription_pancake_language_upgrade` | 1/1 通过 | 19.119 秒 |
| 信任策略 | `trust_policy_upgrade` | 1/1 通过 | 8.170 秒 |

Go 的四个顶层测试通过，覆盖离线钱包升级与运行角色隔离、结构及权限漂移拒绝、不安全输入与运行身份拒绝，以及 COMMIT 回复丢失后的原操作恢复；其中包含三个子场景。整个包的竞态检测运行用时 5.475 秒。Rust 修改范围的格式检查和差异检查通过，Go 修改已通过 gofmt 与差异检查。

账号申诉路径的首次通过结果保持有效。首次 Pancake 执行器路径指向封存目录，被测试在运行升级前拒绝；修正到已核对摘要的实际二进制后，复验该路径和其余未运行路径。初次构建达到执行时限后，复用已有缓存完成剩余构建。原日志、失败结果和后续通过记录均保留在结构化证据及开发机输出目录。

## 来源与清理

实现基线为 [b9489537](https://github.com/TokenNotIncluded/lmm-api-source/commit/b948953712978a297a4b49e34beebd7514b52809)，修复分两笔提交。固定历史执行器来自 `2a0c7bd380f58a553dc903d399255821c1a65d0f`（early58）和 `0510c2455dabcb63e33c719ba52ac189d62612bc`（Pancake67）；执行器摘要、构建命令、各用例退出码及源码摘要见[结构化证据](lmm-upgrade-regression-2026-11-20.evidence.json)。

两次 Rust 自测创建的临时数据库集群均已停止并移除；Go 自测的临时数据库目录无残留。工作使用现有 main 克隆，原始工作区的并行改动保留。

## 本轮边界与后续工作

本轮结果为开发自测。远程 CI 未重跑，独立业务验收、发布、部署、切流及真实付费调用均未执行。既有 CI 的原失败状态与日志保留。

当前仍需推进 Core69 到当前版本的完整历史身份/API Key 导入资格、真实数据与额度对账，以及候选版本的完整发布资格和独立业务验收。当前状态继续维护在[项目进度](../progress.md)。

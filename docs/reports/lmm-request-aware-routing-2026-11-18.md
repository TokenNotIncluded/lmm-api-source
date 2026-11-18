# LMM API 请求能力感知路由迭代报告

日期：2026-11-18（Asia/Taipei）

## 完成结果

已修复首个候选供应商输出上限不足时，整个请求提前返回 `RouteUnavailable` 的问题。源码提交 [1831c35b](https://github.com/TokenNotIncluded/lmm-api-source/commit/1831c35bf7c047c6af59491ce6a4ce1f06a81c08) 已推送到 `main`，基于 `dab41dbb55fa85f1d8879e33d50623e70baa97ee`。

此前，路由先按 `max_attempts` 截取候选，再检查首个候选是否能处理本次请求。这样会把“配置有效”和“满足当前请求”混为一谈：A 输出上限为 128，B、C 为 4096，而请求要求最多输出 129 个 token 时，A 不满足要求便导致请求失败，即使 B 可以处理。

## 实现与行为

本轮先按既有候选顺序调用 `route_for_attempt`，跳过返回 `RouteUnavailable` 的候选，其他错误仍立即返回。建立可用候选集合时，再应用每条路由的发送次数预算。上述例子中，`max_attempts=1` 选择 B；`max_attempts=2` 时，B 出现满足既有重试条件的失败后可以转向 C。A 未发送请求，不消耗发送次数。

变更位于 `services/core/src/model_http/routing_session.rs`。原有协议、计数合同、输入上限、报价语义与重试条件继续约束备用候选；重试沿用同一份冻结报价及资金预留。没有新增依赖、数据库迁移或 UI 改动。

## 新增回归覆盖

三个测试均位于 `services/core/src/model_http/selected_bound_tests.rs`。

| 测试 | 验证内容 |
| --- | --- |
| `selected_bound_request_aware_selection_precedes_attempt_limit` | 可处理请求的首选仍优先；输出 129 时跳过上限 128 的候选；发送预算为 1 和 2 时选择、备用候选及冻结合同正确。 |
| `selected_bound_request_aware_retry_keeps_one_reservation` | B 首次发送后注入未发送成功的连接失败，转向 C；实际尝试编号为 1、2；沿用同一笔预留并按合成用量结算。 |
| `selected_bound_request_aware_rejects_all_incompatible_without_hold` | 所有候选均无法满足输出上限时返回 `RouteUnavailable`；不创建扣费请求，余额不变。 |

## 验证结果与首次失败

| 阶段 | 结果 | 解释 |
| --- | --- | --- |
| 修复前回归 | 1 项通过，2 项失败 | 重试测试复现原有 `RouteUnavailable`；选择测试另有夹具断言问题：空 `fallbacks` 字段实际会被序列化省略，已将断言改为检查字段缺失。 |
| 修复后相关测试集合 | 49 项通过，1 项失败 | 三个新增用例全部通过。唯一失败为已有的真实 PostgreSQL 丢失确认测试，连接环境不符合其 TCP 故障代理要求。 |
| 同源专项复验 | 1 项通过，0 项失败 | 将该用例放到全新私有回环 TCP PostgreSQL 环境后通过；六项共有输入 SHA-256 与上一阶段相同。 |

最终结果是 **49 项通过，加上 1 项专项复验通过，共 50 项跨运行通过**。这不是单次运行 50/50 全绿；首次失败与环境修正均保留在[结构化证据](lmm-request-aware-routing-2026-11-18.evidence.json)。

唯一需要复验的现有用例为 `prepared_contract_real_lost_ack_reserve_start_capture_preserve_one_operation`。其 `lost_ack_pool` 构建 TCP 故障代理，原始隔离环境的连接选项保留了 Unix socket，连接阶段返回 `Io(NotFound)`。专项复验改用绑定 `127.0.0.1` 临时端口、随机 SCRAM 凭据的新 PostgreSQL；未修改该测试或被测业务源码。

### 实际命令与环境

工具链：`rustc 1.100.0 (0ef1bc386 2026-11-09)`。使用离线、锁定依赖构建，单构建任务、单测试线程，开发与测试 debug 信息关闭，增量编译关闭。复用既有 Cargo 构建缓存。

修复后相关测试：

```sh
cargo test --locked --offline --jobs 1 \
  --manifest-path services/core/Cargo.toml --lib -- \
  --test-threads=1 selected_bound_ route_failover_pg
```

专项复验：

```sh
cargo test --locked --offline --jobs 1 \
  --manifest-path services/core/Cargo.toml --lib -- \
  --test-threads=1 \
  prepared_contract_real_lost_ack_reserve_start_capture_preserve_one_operation
```

命令均由隔离驱动提供专用数据库连接，并清除继承的数据库环境变量。测试使用真实临时 PostgreSQL、合成身份、合成渠道与账本数据；未调用真实付费供应商。所有临时数据库停止成功，临时数据库目录已清理。三阶段执行期间各自的源码摘要均保持不变；修改文件的 Rust 格式检查与 `git diff --check` 通过。

结构化证据保留命令、返回码、起止时间、输入摘要及原始结果。记录中的 `source_commit` 是运行时的基线 HEAD；修复后的实际工作树内容由摘要标识，随后进入上述源码提交。完整本地日志位于本工作区 `scratch/route-selection-checks/`，没有将日志位置表述为公开附件。

## 验收边界

本轮完成开发自测与源码推送。独立业务验收、发布和部署未执行；未宣称线上运行版本已经包含本次修复。工作区同时存在的前端登录改动没有纳入本次提交。当前项目状态入口仍为 [docs/progress.md](../progress.md)。

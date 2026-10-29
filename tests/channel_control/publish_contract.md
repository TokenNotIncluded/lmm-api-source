# 事务内 routing 发布：开发接线与自测

当前变更只提供 Core 内部发布原语。接口为：

```rust
PostgresRoutingStore::publish_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    config: RoutingConfig,
    price_version: u64,
    expected: Option<ReleaseVersion>,
) -> Result<ReleaseDocument, StoreError>
```

这是 `pub(crate)` 的 async 关联函数。外层 Core 用例自行开启事务，在该事务中锁定当前有效 Session、账户及完整 persisted policy 并判定相应动作，然后调用；它还负责同一事务的业务审计、回滚或提交。该入口不接收调用者的授权布尔值，也不实施授权。调用方必须在任何错误时回滚；成功返回只表示数据库事务内的候选 release 已封存并指向该版本，提交前仍可回滚，不代表已提交，更不代表任何进程已应用内存快照。提交结果不确定时应先核查数据库，不能猜测重放。

现有公开 `publish` 保持原签名和返回类型：先检查版本参数、开启自己的事务，调用上述入口，提交成功后返回。它本身不增加渠道授权；已有可信内部调用者仍须遵守原边界。入口内在相同事务读取 sealed 价格，使用原 `PriceSnapshot::compile`、`Router::new`、文档编码验证，再调用现有 `routing_publish_config`，由 SQL 完成 active 行锁、版本 CAS、投影和封存。现有发布函数没有渠道审计，fixture 的 audit 表仅用于验证事务组合。SQL 函数/表的原有授权不变；目前没有在 runtime 配置中给出可安全执行渠道写入的数据库能力。

`publish_tests.rs` 被现有 routing 的 `#[cfg(test)]` 叶子包含，计划以独立 PostgreSQL 数据库验证：

- 外层同一事务暂存价格和发布、写入 fixture audit 后故意触发 SQL 错误，回滚后版本、全部路由/价格投影、audit 均不存在，原 CAS 基线仍可成功提交；其他连接在提交前只看见旧 release。
- 两个外层事务以同一基线并发发布，只有一个提交，另一个收到版本冲突；公开旧 `publish` 的冲突、成功及无效版本行为，以及事务入口对无效路由的拒绝。

单独的 `publish_harness.rs` 复用同一测试：它把当前 `store.rs` 原文件作为小 crate 的模块编译，并使用冻结 `liblmm_core.rlib` 的真实 routing 类型、验证和 SQLx；由于 crate 可见性，它不是对 rlib 内 `pub(crate)` 符号的直接调用。两个 private config 项由 harness 小适配器提供，并在测试时检查原 `config.rs` 的确切源码。harness 另有一个独立用例直接调用冻结 rlib 的公开 `publish`。完整 crate 的叶测试才直接调用实际库内入口；编译这个 harness 不等于编译或运行了完整 crate 叶测试。

先执行 space-first，确认输出空间；在共享 `/tmp/lmm-delegate/heavy-build.lock` 及 2G MemoryMax、1536M MemoryHigh、4G MemorySwapMax、CPUQuota=100% 的资源范围内执行。该目录的 `publish_compile.py` 接受冻结目录、rlib/source-pins/compile-config 三个 SHA-256、独立指定的当前 store 源码 SHA-256，以及一个**不存在的私有输出目录**。它验证冻结全部源码 pin、当前基础 routing 源码与 pin、测试输入及 rlib，再链接小 harness；输出编译日志、绑定和二进制 hash。它不编译 Core。冻结库可不包含事务入口，但结果必须记录两份 store SHA 和是否相同；旧 rlib 的公开 API 测试不能证明新的 crate 内私有符号。

编译成功后，同一资源和锁范围内运行 `publish_run.py --out <编译输出目录>`。它创建自己的短 `/tmp/lmm-channel-pub-*` 目录和全新合成 PostgreSQL，关闭 TCP 监听，只允许本用户私有 Unix socket；SQLx 测试创建自己的测试数据库。结束时停止自己启动的 PG；成功才删除本轮 PGDATA，失败保留 PGDATA、日志和结果。不要给它生产或其他人的数据库 URL。运行前保证测试输出目录及旧证据不被复用。若需要完整 crate 的 `#[cfg(test)]` 验证，冻结源码中还必须包含 `tests/channel_control/publish_tests.rs`，而不只是通常的 Core 编译输入。

结果是开发自测，不是独立验收，也不证明 Session/policy gate、密钥生命周期、跨进程快照更新或完整渠道 CRUD 已接通。

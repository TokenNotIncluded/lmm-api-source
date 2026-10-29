# 原生模型性能观测

Core 在共享模型 relay 的真实执行中记录单调时钟采样，以 `core_charging.usage_events` 的 `(request_id, performance-v1)` 唯一键保存。写入遵守原表的串行化事务规则；不改 schema 或 runtime 授权。采样是观测，不参与价格、扣款、退款或恢复授权；写入不确认时不重发供应商、不重复结算，也不覆盖已有事件。

## 指标口径

- 转发耗时：RelayWorker 开始到 execute 结束，包含该阶段的预留、上游和背压，排除入口鉴权及后续资金终结。
- 流式首字：首个非空文本、思考或工具参数 Delta。Start、Usage、心跳、空 Delta 不计；非流式为 null，不猜供应商生成时间。
- 成功：provider_complete、Report Completed、Settlement Confirmed、Completion 无错误同时成立。终态后结算失败或客户端写出失败不能只凭提前的 Report 算成功。
- 转发吞吐：成功样本的输出 token 总数除以这些样本的转发总秒数。不是每条速率的平均，也不是供应商纯生成速度。
- 请求数：已接纳并有采样的逻辑请求。不计未预留的 HTTP 请求；未采样的 charging 请求另列，不伪造零延迟、零 token 或成功。

事件中的 finished_at 是 execute 结束的 UTC 秒。读取在同一只读 repeatable-read 快照中检查当前个人 Session、enabled 路由组和模型可见性，标签来自原请求冻结 price 中的 canonical model/group，不依赖 Key 当前组。匿名、API Key、团队和失效 Session 拒绝。撤组会按现有身份规则撤销凭据，不能把已撤销 Session 继续当有效查询身份。

## 读取合同

`GET /api/perf-metrics/summary` 返回 data.models 与 data.aggregate。
`GET /api/perf-metrics?model=<canonical>&hours=24&bucket_seconds=3600` 返回模型的 data.groups、分组 series 与 data.aggregate。

hours 默认24，范围1至720；bucket_seconds 默认3600，只接受60、300或3600。UTC半开窗口的上界取读取时下一秒，以纳入已提交的当前秒采样；下界为上界减 hours。最近三项成功率使用最近三个非空桶。平均值由真实分子、分母计算，空集合为 null；十进制字符串计数避免不必要的浏览器整数舍入。读取流式处理结果，不设一万样本截断；现有只读事务超时仍有效。

每次返回 availability=partial，coverage 明确 historical_imports=false、sampled_requests、missing_samples、窗口和指标口径。历史性能、非共享 relay 入口及进程死亡时丢失的观测不能被这批原生样本冒充。完整全站性能、历史迁移、所有模型入口、浏览器和S11仍分别验收。

## 本地开发验证

先按 space-first 检查空间，使用已有固定工具链/依赖图编译并冻结 Core、库、source-pins 和 compile-config；不使用未提交的并行改动替代固定输入。开发检查：

```bash
python3 -B tests/performance/selftest.py \
  --freeze /absolute/frozen-build \
  --source /absolute/lmm-api-source \
  --output /absolute/new-short-private-root
```

通过有2GiB内存上限和共享 heavy-build 锁的用户单元运行。output 必须不存在，PG Unix socket 全路径不得超过107字节；新建短目录，不能删除旧失败证据后复用路径。检查启动真实受限 Core、隔离 PostgreSQL 和带计数/延迟的 TCP JSON/SSE 供应商；owner 用真实 Identity/Ledger API 创建合成测试数据。测试原始凭据/日志只留私有目录。成功、失败、输入版本及独立验收分别记录，完成后停止本轮进程并清理成功临时副本。

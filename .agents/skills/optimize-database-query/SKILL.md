---
name: optimize-database-query
description: "优化 LMM API 的 PostgreSQL 读写路径。用于慢列表、N+1 查询、重复扫描与索引评估，要求同时验证结果语义和数据库实际执行的工作量。"
---

# 用可复测的负载优化数据库

选一个用户操作，记录它从入口到 SQL 的调用次数、查询参数、排序、页大小和表规模。Core 与 Extensions 各自拥有自己的数据。先看同一仓库里的建表、升级和查询代码，确认旧库也能使用改动。

1. 用属于本次测试的空实例和合成数据复现。记录 PostgreSQL 版本、行数、键分布、统计信息与查询参数。按[质量指南](../../../docs/quality.md)核实实例归属。日志只记查询形状和计数。
2. 对 SELECT 比较同一份数据上的 `EXPLAIN (ANALYZE, BUFFERS)`。保存扫描类型、实际行数、循环次数和缓冲区计数；首轮与热缓存分开标记。多次小查询先计应用请求实际发出的 SQL 数。单次耗时只用作该夹具的观察值。
3. 改一个已定位的原因。对取单行的判断先核对能否停止扫描；列表按它真实使用的过滤、排序和分页评估索引；批量读取保留缺失行、重复键和返回顺序。新增索引同时核对写入代价、部署时锁和现有库的升级路径。
4. 用相同参数比较改前、改后结果。保留隔离范围、权限、空集合、分页边界和事务可见性。先记录能区分两种实现的计数或计划变化，再报告耗时；生产表现需要相应环境的证据。

PostgreSQL 的 [EXPLAIN](https://www.postgresql.org/docs/current/using-explain.html) 和[索引使用检查](https://www.postgresql.org/docs/current/indexes-examine.html)说明了如何解释扫描与成本。涉及写语句时，`EXPLAIN ANALYZE` 会真实执行语句，只使用可丢弃实例。

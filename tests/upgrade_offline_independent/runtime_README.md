# v6 升级后实际 runtime 独立验收

只负责 `runtime_*`，不改产品、其它 owner 的 v5 文件或旧浏览器 runner。
最终候选固定 compile-r2，Native SHA256
`dea60909b00bed72db09772200db99233b9eb68bdb9a62197aa38288786d71aa`。
1308 个冻结输入逐项对照 git blob，匹配提交
`fcb29cd98950053ab3a76e29548d8752366baa22`；不把 r1 或旧 2d Native 计作新 runtime 结果。

复用 `scripts/test-upgrade-offline.py` 的 PG/systemd 操作方式，但独立实现准备与
SQL/HTTP/进程 oracle；不导入或调用作者 `--verify-runtime`、runtime_checkpoint/exercise。
实际旧 Core init-db/create-first-owner 创建 v6/27 库，再由新 Native upgrade-offline。
没有 v5 数据伪造、Web 构建、浏览器、依赖安装、重新编译、生产访问或付费调用。

独立核对升级已 committed，数据库关闭、旧服务停止且 masked、外部 history committed。
之后人工修改隔离夹具的 LOGIN/CONNECT/HBA/ALLOW_CONNECTIONS；显式记录
`product_finalization=false`，不视为产品已完成恢复运行、也不授予生产开站权限。

验收组：

1. 真实 v6 → 新 Native offline upgrade，原密码行/Notice保留，目标catalog绑定。
2. 夹具人工开放，runtime身份无owner/高权限/CREATE。
3. offline_upgrades和console_revisions表级/列级SELECT及所有DML授权均为无；
   runtime真实连接各自执行SELECT、INSERT、UPDATE、DELETE、TRUNCATE、ALTER及schema CREATE，
   必须SQLSTATE42501，全部事务回滚。
4. 实际进程MainPID及/proc/exe SHA；原密码登录L6 Session，Notice HTTP保存/读回/SQL一致。
5. 撤销公开component_upgrades SELECT，实际serve必须MissingPrivileges；恢复原SELECT。
6. 撤销一个非显式特例billing表SELECT，实际serve仍MissingPrivileges；恢复原SELECT。
7. 两张私有表分别误授表级SELECT、单列SELECT，四次实际serve必须UnsafePrivileges；
   每次撤销并重新核对最小角色，不保留误授权。
8. 误授数据库CREATE，实际serve必须UnsafePrivileges；撤销并核对。

每组失败立即停后续组；不放宽断言。私有误授权仅是明确记录的负例故障注入，
不增加产品或最终runtime元数据权限。旧库密码不重置；禁止把HTTP成功mock计作通过。

运行前使用space-first检查目标空间与同前缀残留。实际调用：

```bash
python3 -B tests/upgrade_offline_independent/runtime_launch.py \
  --output /home/lightjunction/Documents/Codex/ur1030NEW
```

Root必须新建，存在即拒绝；所有源码/二进制副本/PG/日志/凭据都位于私有Root。
仅临时systemd单元定义位于用户runtime目录，结束删除。
所有PG、旧Core服务、新CLI和runtime进程位于同一个受限aggregate slice；
共享flock `/tmp/lmm-delegate/heavy-build.lock`，2GiB admission，
Max2GiB/High1536MiB/Swap4GiB/CPU1/Tasks256。锁最多等20分钟，每10秒记录等待。
成功停止并清理PG、源码/Native副本和凭据，保留报告/runner/命令/哈希；失败停止进程、
重新关闭夹具访问并保留全部原件。同类失败两次停止。

所有PASS只属于独立合成v6升级后runtime权限/身份/Notice范围，不覆盖v5、全站、
恢复运行产品协议、生产、发布、部署或资源S11。开发检查与独立验收分开报告。

## 2026-10-30 实际结果

独立 `ur1030a2` 八组全部 PASS，launcher exit 0。实际命令与上方相同，
`--output /home/lightjunction/Documents/Codex/ur1030a2`；复现须使用不存在的新 Root。
未使用作者开发 PASS 作为任何独立断言，也没有重跑浏览器或 v5。

- 新 Native 实际进程 PID 967681，`/proc/967681/exe` SHA 与上述候选完全一致。
- 原密码登录为 L6 session；Notice PUT、GET 与 SQL 持久值一致。
- runtime 实际角色为 `lmm_core_runtime`，无 owner 成员关系、超级权限、DB/schema CREATE；
  六张公开合同及全部 17 张 billing 表可读，两张私有表无表级/列级权限。
- 13 条真实私有访问/写入/DDL探测全部 SQLSTATE 42501。
- 撤 `component_upgrades` SELECT、撤一个非特例 billing 表 SELECT：实际 serve 均 MissingPrivileges。
- 两张私有表各自整表/单列 SELECT 误授共四次，另加 DB CREATE 误授一次：
  实际 serve 均 UnsafePrivileges；全部恢复后再次验证角色。
- 共享锁与资源上限实际生效；本轮 admission 2,769,908 KiB，
  aggregate memory peak 549,851,136 bytes，swap peak 2,232,320 bytes，pids peak 29，OOM 0。

首轮 `ur1030a1` 保留 INCOMPLETE：真实升级已 PASS，人工恢复后本地 postgres oracle
仍被 HBA 围栏拒绝，`027-sql.log` 为真实连接拒绝。其余依赖组未运行。
这是独立夹具缺少管理连接放行规则；仅在下一轮夹具增加本地 postgres 管理规则，
没有给 runtime 增加元数据权限、修改产品或放宽断言。首轮源、runner、日志、DB 完整保留。

两轮所有 Core/PG 已停止，夹具重新封闭；成功轮的 PG、源码/Native副本、凭据已清理。
临时 systemd 单元定义已移除，空 aggregate slice 经外部只读核对无进程后停止。
失败现场保留 272,612,140 文件逻辑 bytes；此数不是本轮释放的物理空间。
没有测量并发环境中的实际块释放量，不作估算。

私有证据位于 `/home/lightjunction/Documents/Codex/ur1030a2/`：
`REPORT.md`、`results.json`、`actual-process.json`、`http.json`、`commands.json`、
`private-SQL-probes.json`、`cleanup-audit.json`、`hash-manifest.json`。
实际执行副本保留在 `runner/`，公开最终三文件快照在 `deliverables/`；
执行结束后只补充本 README，两个 Python 文件与 PASS 轮执行输入一致。

输入清单 SHA256：`ac32f15464d42b4cf7f164e510af79d9e3183dc5fb56ec68cf2c135bf298f9e8`。
Native manifest SHA256：`47ad36927c16ccc4237cf16c9889705ae4d53db184c77aaac4a264c4e5203078`。
升级后 catalog SHA256：`557d9775042b94356f752ae4993bc73be645e9956e9fe1110ed5aa1cd531ee2d`。
前驱实际 Core SHA256：`6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31`。

本验收不声明产品已实现 finalization，也未运行物理备份恢复、v5、Web/浏览器、
全站验收或生产部署。上述人工恢复访问仅属于私有隔离夹具。

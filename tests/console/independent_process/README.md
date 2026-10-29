# Console 独立实际进程验收

状态：2026-10-29 attempt-05 已实际运行，10 组 PASS、退出 0；实际 Core SHA-256
`8d760cf7012eb4436001469406e4afeb6b6161001a64dd23d67ae824019692b7`。
完整报告：`/home/lightjunction/Documents/Codex/2026-10-29-console-independent-attempt-05/REPORT.md`。
此前锁阻断及三轮夹具错误原始记录保留；不把它们改写为产品通过。
本目录仅独立测试，不修改产品、既有断言、schema 或授权脚本，不提交/推送。

输入基于主线程交付：

- `/home/lightjunction/Documents/Codex/2026-10-29-console-formal-50jdzil7/source`
- 同目录父级 `source-pins.json`（1293 文件）、`freeze.json`
- 同目录父级 `lmm-core`，最终 SHA-256 已由主线程交付并在执行前后核对。
- 基准 main `a853ea8f0186c948a62206005931efc1c47004d8`，identity v6 / 27 组件 / 支付；
  Console 新装、options routes/writes、runtime 检查及 Console owner-only metadata。
  不引入 control-v2，不用新装冒充存量升级。

来源已阅读：源码与父仓库 AGENTS、父 STATUS，以及
`/home/lightjunction/Documents/Codex/2026-10-29-console-delivery/DELIVERY.md`、`TEST-COMMANDS.md`。
历史 Router TCP、旧 v5/26 升级、邮件/支付实际进程的通过均不继承到本验收。

## 执行与冻结

不构建、不复制源码、不使用 `lib-options-tests` 或 `restricted_console_acceptance`。
实际 `lmm-core --config … init-db`、`create-first-owner` 和 `serve` 均使用同一二进制。
密码从私有 stdin 传入，管理员通过真实 TCP `/api/user/login` 登录。
测试与产品输入分别记录 SHA-256；哈希用于绑定输入，不单凭哈希证明编译来源。
主线程的构建命令/日志与最终产物交付构成来源证据，版本是完整 Cargo dev 特性图的
实际 main，不能标为 release。

仅在最终 hash 交付、共享构建锁释放、再次按全局 space-first 检查后运行。
新的 2 GiB admission 是本次有界开发路径；不修改旧 3 GiB 拒绝记录，不冒充 S11。
脚本内部使用非阻塞共享锁，检查 cgroup，不支持绕过参数：

```bash
cd /home/lightjunction/Documents/GITHUB/lmm-api/generated/lmm-api-source
console_freeze=/home/lightjunction/Documents/Codex/2026-10-29-console-formal-50jdzil7
console_sha=替换为主线程最终交付的64位SHA256
console_evidence=/home/lightjunction/Documents/Codex/2026-10-29-console-independent-attempt-01
XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
systemd-run --user --wait --pipe --collect \
  --unit=lmm-console-independent-attempt-01 \
  -p MemoryMax=2G -p MemoryHigh=1536M -p MemorySwapMax=4G -p CPUQuota=100% \
  python3 -B "$PWD/tests/console/independent_process/run.py" \
    --source "$console_freeze/source" --core "$console_freeze/lmm-core" \
    --core-sha256 "$console_sha" --pins "$console_freeze/source-pins.json" \
    --freeze "$console_freeze/freeze.json" --evidence "$console_evidence"
```

禁止沿用已有输出目录。失败后保留 attempt，先定位是产品还是测试夹具；不覆盖失败，
不删断言改绿。测试夹具若需修正，记录原失败、新测试 hash、修正原因并使用新 attempt。

## 测试矩阵与证据要求

| 组 | 操作与硬断言 | 证据 |
| --- | --- | --- |
| 冻结/资源 | 核对所有 source-pins、binary SHA；取得 heavy-build.lock；MemAvailable ≥ 2 GiB；cgroup Max2G/High1.5G/Swap4G/CPU1 | binding-before/after、资源快照、命令退出码 |
| 新装/运行 | 新 PG18，私有 Unix socket、PG 无 TCP；非 superuser installer SET ROLE 到 NOLOGIN owner；实际 CLI init-db；正式 grant `enable_console=true,enable_web_identity=true`；非 owner runtime 启动实际 main | PG system identifier/data directory、v6/27、runtime role 属性/连接身份、PID 与 `/proc/PID/exe` SHA、ready capabilities |
| 登录与拒绝 | 真实管理员密码登录；普通用户、管理员 API Key、匿名、撤销后的管理员 Session 在 GET/PUT/bulk/validate 均拒绝；精确状态/error code、success=false、no-store；状态与 Console 审计不变 | HTTP 结构事件、前后快照 digest、同事务 actor/credential 绑定 |
| 内容保存 | Notice/公告保存；两别名同时更新；读取一致；no-op 不变 revision/timestamp/audit；validate 不写；坏 JSON/类型/不支持键/混合批次/别名冲突/stale CAS 拒绝且全不变 | 状态码、saved、数据/审计前后精确断言 |
| 并发 | 两个独立 TCP 请求同步起跑：Notice 和公告跨别名 CAS；每轮恰一 200、一 409；仅一次审计、每个实际更改键 revision +1 | 两个响应和 winner 内容/审计一致 |
| 回滚 | 仅在私有夹具撤销 audit INSERT；真实 HTTP 多键写入返回 503 STORAGE_UNAVAILABLE，所有 options/revision/timestamps/audit 不变；用完整正式 grant 脚本恢复 | 前后快照、正式授权脚本命令、错误体 |
| COMMIT 回执丢失 | 代理转发真实 Console COMMIT，见 PG `CommandComplete(COMMIT)` 和 `ReadyForQuery(I)`，抑制回应并关闭连接；HTTP 必须 503 COMMIT_UNKNOWN，不许声称 rollback/未提交/成功；另连接确认目标行与审计已提交；不自动重试写 | commit-fault.json、代理结构事件、独立 SQL 与随后 HTTP 读取 |
| 重启 | 同一正式 Core 停止/重启、同一 runtime 账户；已保存数据与审计保持 | PID/exe binding、snapshot digest、HTTP 读取 |
| SQL 信任边界 | runtime 直接 DML/DDL/SET OWNER/metadata 读拒绝；函数不支持键拒绝；允许函数直接 SQL 能写非法公告 JSON、不会写 HTTP 审计/同步另一别名，单独记录 | sql-trust-boundary.json；不是数据库强制用户审计的 PASS |
| 不泄秘密 | 只用随机合成密码/凭据/隐藏选项；options 响应不得回显已知秘密或 SQL/连接细节；Core 日志检查；分享证据不保存原始 HTTP/SQL payload、cookie、token | 固定错误标签、脱敏日志；私有目录 0700/文件 0600 |

每组 `PASS/FAIL/NOT_RUN` 明列，整体只有全部执行通过、输入未漂移且清理成功才 PASS。
资源/锁失败不得称产品通过。独立进程报告必须区分：测试脚本自身检查、正式 main
业务验收、推送/发布/部署（本任务禁止）、实际浏览器（未执行）。

## COMMIT 注入机制

`pg_fault.py` 是 loopback TCP → 私有 PG Unix socket 的透明协议代理。
只接受固定 runtime/database 的 PostgreSQL v3 plaintext startup，不记录 SQL 参数或秘密。
记录 prepared statement 名称到 Console writer 的映射，因此 SQLx 复用 prepared statement
时仍能识别。只对 armed 的一次 writer 事务 COMMIT 抑制回应；其他提交正常转发。
不伪造数据库成功，不杀 PG，不替换产品事务实现。若未观察到完整提交确认或遇到
不支持的协议分支，测试失败/未覆盖，绝不以普通掉线代替该场景。

`test_pg_fault.py` 仅检验注入器自身的透传/定位/抑制逻辑，使用合成协议帧；它不是
PostgreSQL 或 Core 验收。可单独轻量运行：

```bash
python3 -B tests/console/independent_process/test_pg_fault.py
python3 -B tests/console/independent_process/run.py --help
```

## 明确边界与清理

runtime 是受信任 Core application，与既有 identity writes 一致。
HTTP 保存保证用户鉴权、内容校验、CAS、审计事务原子性；持有 runtime DB 凭据的程序
可以直接调用允许函数，不能据 HTTP 通过宣称数据库强制所有用户审计。直接 SQL
测试安排在 Core 停止后、最后执行，观测其真实能力，不污染之前 HTTP 用例。

只创建本地合成账号。无生产、DMIT、paid calls、CI、模型供应商、Go 支付进程。
未覆盖存量升级/桥接/旧库围栏、浏览器、支付、完整 A01–A10/S11；这些仍由原 owner 负责。
成功后停自己 Core/PG/代理并删除自己的临时 DB/秘密配置/socket；保留小型脱敏证据。
失败同样先停进程，私有夹具作为待诊断材料保留，不删除他人缓存或旧失败记录。

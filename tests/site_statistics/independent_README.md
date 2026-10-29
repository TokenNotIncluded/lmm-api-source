# 独立 site-statistics 验收

最终独立结果：**si1030a10，R7当前组合20/20 PASS、exit0；si1030p4，真实首管回归1/1 PASS、exit0，共21组。** 全部新跑，未复用a8/R6、a5/旧writer或R3结果；已废弃的epoch±1/2不在矩阵中。浏览器由James准备、主线CUA执行，本侧没有浏览器PASS。

**R6首管503已在R7独立关闭。** p2的正式CLI/密码登录成功但财务503原件继续保留；p4用同一零值业务断言，实际得到200。R7只在reader中增加真实`installation.first-owner`审计动作；actor/target匹配及imported检查不变。a10的缺来源503、历史映射用户preseal request后来settled503仍实际通过。

## 执行输入

| 输入 | SHA256 |
| --- | --- |
| 实际 Core | `d2bf71c19b126e40cb3db85612c1304c38433cb04c048fe4893d8353507f323f` |
| 同轮 rlib，独立计算 | `18e9e246a4da3fca282de07d77b9170a09a2c60ceed29f45809686cba73732ce` |
| 1316 source pins清单，全项核对 | `752448d60c095a5473f2ff43505948964e68f5ae7060cbaf92f41b4321429233` |
| 实际 schema-manifest stdout | `207bc6bf3f39dc9803177e6cebda491150fb66370a5ff3fc37b26b863cfec97b` |
| site_statistics.sql | `9bca87cc0cdc5ba96efb5c8bc7ef737684d74fcde257adba713a5951ebf63eb1` |
| owner writer | `386eb4923757b4e764480a2c8c0d90ff9359cbf7a04bfb69e630a5acd2046d9b` |
| 干净 R7 core_schema.rs（与R6相同） | `9ebce14ac2856dfb8645895ce2f13b0b6d947cb911c54f5664d8bd18062ca8f1` |
| 独立金额 oracle | `07d2298dc533f820161ca50b8972c01e1f83d7023108eae7c0a4d181b13b3b1c` |

候选目录 `/home/lightjunction/Documents/Codex/2026-10-30-site-finance-cnjakvgy/compile-r7`；catalog `5521fb8224fbdfc994421f0fe04756df99a4e341ceec78b6302882a1f145d2d0`，28组件/175表/22序列。owner依赖及干净Core文件从固定输入复制，未读取工作树console WIP作最终来源。复用的PG机械代码、身份seeder以及本侧所有runner/helper在锁前冻结，详见a10的binding.json。

实际命令（在源码仓库执行）：

```sh
python3 -B tests/site_statistics/independent_run.py \
  --root /home/lightjunction/Documents/Codex/si1030a10 \
  --candidate /home/lightjunction/Documents/Codex/2026-10-30-site-finance-cnjakvgy/compile-r7 \
  --core-sha d2bf71c19b126e40cb3db85612c1304c38433cb04c048fe4893d8353507f323f \
  --rlib-sha 18e9e246a4da3fca282de07d77b9170a09a2c60ceed29f45809686cba73732ce \
  --source-pins-sha 752448d60c095a5473f2ff43505948964e68f5ae7060cbaf92f41b4321429233 \
  --import-tree /home/lightjunction/Documents/Codex/sfp1030a1/importer-r6-fixed \
  --import-sha 386eb4923757b4e764480a2c8c0d90ff9359cbf7a04bfb69e630a5acd2046d9b \
  --seal-regression --execute --full-current
```

复现必须换未存在的新短root。`--full-current` 禁止复用旧组；`--resume-seals` 和旧R3复用模式仅保留失败调查历史，不是最终20组的运行方式。外层真实argv在 `sfp1030a1/r7-a10-full-actual-launch-argv.json`，systemd命令在a10的launch.json，各阶段SQL/HTTP/命令/响应在logs。

## 正式首管回归

p4在独立空库依次运行实际R7 `init-db`、`create-first-owner`，通过私有stdin给合成首管设置密码，再使用canonical grant启动受限runtime。真实`POST /api/user/login`返回200，得到的个人Session调用`/core/v1/identity`返回200/L6，再调用财务接口返回200。未手工插入users、密码、Session或审计。

实际审计为`installation.first-owner`（actor=1、target.user_id=1）及`session.web_login`（actor=1、target.user_id=1、session_id=1）。SQL独立确认只有该用户，opening/request/payment均0；财务独立预期为used/balance字符串`"0"`、500000 credits/USD、空currencies/virtual_units、三种订单计数均0，不从实际返回倒算预期。MainPID2417801的exe SHA匹配R7。

```sh
python3 -B tests/site_statistics/independent_first_owner.py \
  --root /home/lightjunction/Documents/Codex/si1030p4 \
  --candidate /home/lightjunction/Documents/Codex/2026-10-30-site-finance-cnjakvgy/compile-r7 \
  --core-sha d2bf71c19b126e40cb3db85612c1304c38433cb04c048fe4893d8353507f323f \
  --source-pins-sha 752448d60c095a5473f2ff43505948964e68f5ae7060cbaf92f41b4321429233
```

复现同样需换新root。此runner不链接rlib/helper，密码与Session只来自真实产品流程；它是API回归，不是浏览器。p4与a10串行，原2GiB admission不变；p4 MemoryPeak304025600 bytes，Tasks96，其余内存/CPU限制与矩阵相同。成功后Core/PG均停止，删除56832001逻辑bytes的PG与临时凭据，保留CLI、身份、审计、HTTP和哈希证据。两轮合计清理887918520逻辑bytes。

## 独立预期

旧合同固定父仓库传统提交 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85`。oracle只导入标准库，使用人工分类、整数和Fraction；开发reader、归一化、分类函数及作者自测输出都不作预期值来源。owner importer是被测对象。

基础历史用量653+42+0=695 credits；余额70+40−5=105，reserved20/affiliate700另列。原币种gross/refund/net（micros）：CNY2300005/3/2300002、USD2000001/1000001/1000000、LDC3300000/1000001/2299999；确认4、未知4、异常2。Native USD123 cents/退23 cents增加1230000/230000/1000000 micros，剩余500000 credits另算；不从余额推现金。

真实本地模型输入10/output5；TokenRates单位是micro-USD每百万tokens，费率2000000/4000000，500000 credits/USD，admin折扣9/10：

```text
ceil((10×2000000 + 5×4000000) × 500000 × 9 / (1000000×1000000×10)) = 18
真实 Charging.refund(7) 后净用量 = 11 credits
```

请求封存helper使用公开legacy Price：1000000/2000000 credits每百万tokens，无trust entitlement，因此同一10/5向量为20 credits。这是另一份明确的合成输入，不与Gateway的18混用。所有状态通过真实Identity/Charging API生成，HTTP统计由实际受限Core处理。

## 20组全部实际通过

| 组 | 通过事实 |
| --- | --- |
| 新装及绑定 | 实际init-db、28组件、canonical grant、真实受限serve及MainPID/exe哈希。 |
| 缺来源 | 有opening但无finance来源，503且不返回可信总额。 |
| owner导入 | 人工源事实经过实际writer plan/owner SQL；逐行原币种与归一化独立核对；明确COMMITTED后才精确NOOP。 |
| DTO/权限/只读 | admin/root成功；普通Session/key/匿名/revoked拒绝；每次财务HTTP前后受保护表和序列不变。 |
| runtime ACL | 新3表/view可读；写入及私有meta表/列读取实际拒绝。 |
| Native付款 | USD原金额、已退金额及重启读回一致。 |
| source行数 | 宣称数量与实际行数不符，503。 |
| 未完成退款 | 已验退款尚未按原ID应用，503。 |
| 退款金额 | 同原refund ID的已验金额不符，503。 |
| 退款累计 | refunded_minor与provider_refunds累计不符，503。 |
| Credit未入账 | 已验Credit但无credit journal，503。 |
| Refund未入账 | 已验RefundSucceeded但无credit journal，503。 |
| 实际usage/refund | 1次本地TCP供应商，真实settled18/refunded7/net11，重启不重发。 |
| 第二来源 | owner prepare/apply拒绝不同sourceKey，DB唯一索引23505，状态不变。 |
| 超i64 | 1031×9007199254740991=9286422431637961721；实际owner导入、SQL numeric、HTTP字符串及账本守恒一致。 |
| 旧intent封存 | CACHE16实际分配1、17；live事务被拒绝；关闭后floor32。旧id1即使created_at晚于dump-start，后入账仍503。 |
| 新intent与NOOP | source后新id33即使created_at等于dump-start秒仍成功；NOOP不重采floor32。 |
| 序列前提 | step2/cycle实际HTTP503；step2/cycle/seqLast低于现有id的owner snapshot全部拒绝。 |
| 历史request | 映射用户8的preseal pending不计消费、release不计消费；后settled仍503。 |
| Native-only request | 映射外用户7真实bootstrap审计、账户早于合成dump；preseal消费保留。source后用户8新request计入；NOOP保持原封存ID集合。 |

Source plan保持12字段；server floor与完整opaque request ID数组只在target seals。请求集合不按时间、字符串大小或前缀猜测。实际新旧request的authorized秒本轮不相等，**不声称真实同秒request已验**；同秒新intent已验。

## 资源、清理、执行与报告哈希

a10共享 `/tmp/lmm-delegate/heavy-build.lock`，MemAvailable≥2GiB admission；MemoryMax2147483648、High1610612736、SwapMax4294967296、CPUQuota1s、Tasks256。MemoryPeak **1073061888 bytes**。只链接3个小helper，不重编Core、不安装依赖。

共32次实际Core启动，首MainPID2431715、末2447922；每次 `/proc/PID/exe` 都匹配R7 SHA，数据库、runtime角色、配置哈希均逐轮记录。PG只监听私有Unix socket，HTTP和合成供应商只走127.0.0.1。0生产/DMIT/真实账号/付费调用。

成功后Core/PG均停止、临时unit移除，清理 **831086519逻辑bytes** 的PG、Core/helper副本和临时凭证；不是按文件大小推断物理回收空间。输入、SQL/HTTP日志、截图以外的本任务证据及哈希保留。没有浏览器截图。

a10的4份业务runner/oracle/helper与a8逐字节一致；first-owner runner与p4执行副本一致。执行时binding与runner/内文件不可改。最终public README是执行后报告，另复制到post-report并写final-deliverables.json，分别列execution与post-report SHA。源码提交由主线处理；本侧未提交，不把README追加内容伪装成执行输入。

## 原失败和未验范围

- a1：UTC/+08时间表示不同导致fixture精确绑定拒绝；a2：克隆库未继承PUBLIC CREATE/TEMP撤权；a3：费率误当2/4 credits/token，错算36，正确18。均保留原失败，属于不同fixture问题。
- a4：R3 14+3组通过，保留历史绑定；旧epoch合同后来废弃，不计本次结果。
- a5：R6 12组通过后发现owner guard产品缺陷：普通owner看其他角色backend_type为NULL，漏掉live客户端。d2独立PG证实同一PID在观测前后仍取17/18，未授监控权限；d2诊断PG已清理。修复由作者完成，writer386…改用公开datid+pid；a8实际复验通过。
- a6、d1、p3及a9：在原2GiB门禁处停止，未启动PG，不记产品失败。a9实测MemAvailable2058112 KiB，低于2097152 KiB；无业务组执行。其原binding、launch和资源记录保留，CLASSIFICATION.md明确资源失败。哈希原已流式读取，没有以改算法或门槛取得通过。a7补跑的5个seal/request组通过，但其基本组仍是旧writer证据；a8曾完成R6全新20组，但随后p2发现首管缺陷。a8原结果封存；最终a10在R7再次完整20组，不依赖任何复用。
- p1：最小密码复现脚本函数名遮蔽http模块，HTTP前停止；p2仅修该fixture错误，实际确认R6 **installation.first-owner财务503**。正式密码登录200、身份200/L6，只有该管理员、opening/request/payment均0。MainPID2148236绑定R6；原DB/凭据/响应已停止保留。p4在R7上已实际关闭此Core问题；原R6失败状态不改写。

本侧20组使用合成完整三用户源、合成identity/opening前序事实，调用实际writer SQL接口；**没有声称运行完整archive CLI、canonical首次identity/opening迁移或Native先存后首次旧站导入全链**。真实bootstrap审计不等于完整旧档案证明；公开首次导入仍fresh-only。作者CLI/升级/ACKloss与ProB helper/局部PG结果不算本侧独立通过。

未验：SourceFinance正式CLI完整文件输入链、真实迁移材料、27→28升级、COMMIT ACK丢失、真实支付签名和付款/退款状态机、缺失sealed request破坏性注入、真实同秒request、VIEW篡改/私有meta误授权启动拒绝、实际浏览器。本侧Native付款/已验事件是明确合成SQL事实，资金变更走真实post_ledger；真实模型和Charging退款另有实际执行证据。

ProB更正报告31已撤回旧G1/G2时间缺陷及cutoff补丁建议；本侧保留旧结果但不恢复那些时间预期。完整source map、真实creator与实际writer阶段分别报告。浏览器由James的独立PG/Native/proxy准备、主线CUA控制；本侧静态包533文件哈希核对不能替代浏览器验收。

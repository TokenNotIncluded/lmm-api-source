# 旧充值的未决支付义务

[资金 cutover planner](../scripts/legacy_cutover_money.py) 的钱包阶段读取本机冻结旧库，为仍可结算的充值生成安装 owner 专用的非可用资金回执。它不联系支付渠道、不创建新付款、不增加余额，也不代替支付回调迁移。

## 可结算状态不能只看 pending

旧版 `CompleteExternalTopUp` 的对应容量规则接受两类订单：

- `status=pending`；
- `status=failed`、`payment_provider=waffo_pancake`、`failure_reason_code=checkout_timeout` 同时成立。

第二类只是本地结账超时，不能据此断言支付永远不会到账。提取使用 `top_ups_payable`，不能复用只含 `top_ups_pending` 的旧导出假装完整。回执保留原 `failed` 状态、原行及金额/币种字段，`settlement_state=unknown`、`spendable=false`、`spendable_units=0`；不会把失败状态改成成功或再次给钱包记账。

旧用户已删除时，原行留在私密来源导出并计入 `unrepresented_payable_topup_orphan_rows`，不得改绑新账户、创建可用资金或标记结清。仍需单独完成 owner 隔离保留与处理决定。已有冻结阶段的输入不能原地换成新提取结果重放；完整演练必须在新库按顺序重新执行，并保留原失败和旧输入。

## 折扣名额必须关联订单生命周期

折扣码 TTL 到期不代表名额可复用。旧规则为：

- `reserved` 预留仅在对应订单明确关闭时释放；订单缺失或状态未知继续保留。
- `released` 预留若对应订单仍可结算，也占用名额。
- 没有预留记录、但仍可结算且引用折扣码的旧订单同样占用名额。
- `consumed` 已反映在 `used_count`，不重复计算。

明确关闭仅包括 `expired`，以及不属于上述 Waffo 超时恢复路径的 `failed`。可空失败原因不能凭空补成已关闭。名额核对不改变折扣定义、有效期、用户归属或最低购买额，不启用原生折扣功能。

## 本地证据与剩余门槛

2026-11-04 的冻结旧快照有 13 笔 pending 和 177 笔仍可结算的超时充值。新增本地演练保留 174 笔有映射的超时回执，3 笔已删除用户记录单独隔离；174 笔完整原行、来源 hash、owner、状态和金额均读回一致，严格重放不新增，余额及账本条数不变。旧 SQL 与 Python 独立计算均为 8 个占用名额，其中 5 个对应 released 预留、3 个没有预留记录。

私密证据位于 `~/Backups/lmm-full-rehearsal-20261104/discount-lifecycle/`。这只适用于该冻结快照、本地 owner 事务和对应源码，不证明最终快照、完整新库演练、真实到账或独立验收。

[统一核对器](../scripts/verify_legacy_cutover_rehearsal.py) 比较有映射的超时义务并保留孤儿记录、折扣容量及旧回调未验收的 gap；有缺口仍为 `SCOPED_PASS_WITH_GAPS`，退出码 2。原生折扣报价与原子预留、最后名额并发、旧签名回调的精确一次入账、晚到账消费原折扣名额，以及原支付币种/金额校验继续单独完成；保存回执不能代替这些业务代码。

## 保存逐单原结算观察值

[来源导出](../scripts/export_legacy_settlement_basis.sql) 在本机恢复库的只读事务中保存所有可结算订单、完整重基母计划及额度校准配置。`money` 按原 numeric 文本导出，避免 JSON 浮点数损失。文件含支付引用与金额，必须只存于私密备份目录。

[原函数捕获 fixture](../scripts/fixtures/legacy_settlement_basis_capture_test.go) 只用于父仓库固定历史提交 `3948a0a1ee1cefdd016a7aa45a11851545bb4b85` 的独立源码副本。将它复制到该副本的 `apps/api-go/model/`，编译测试二进制后，在禁用外网的 network namespace 内执行唯一的 `TestCaptureLegacySettlementBasisOffline`。不编译成新架构运行程序，不向新 Go 扩展提供 Core 数据库凭据。

fixture 要求本机 `~/Backups/` 下的 socket、端口 55436、名称以 `lmm_legacy_quote_oracle_` 开头的专用旧库副本。`LMM_LEGACY_ORACLE_SOCKET`、`LMM_LEGACY_ORACLE_DATABASE`、`LMM_LEGACY_ORACLE_EXPECTED_ROWS` 和 `LMM_LEGACY_ORACLE_OUTPUT` 必须显式提供；输出文件不可已存在。它调用原 `VerifyCreditUnits` 的只读路径、`pendingTopUpSettlementQuotaTx` 和 `expectedTopUpAmountMicros`，随后回滚观察事务。仅原 `ErrInvalidTopUpQuota` 计为拒绝，存储等其他错误必须失败，不能伪装为业务拒绝。保留完整源码、捕获 fixture、二进制、运行日志、来源与副本一致性证明及指纹。

[事实 preparer](../scripts/prepare_legacy_settlement_basis.py) 接受 `--input`、`--oracle`、`--archive-sha256`、`--expected-rows` 和私密 `--output`。oracle 文档绑定原提交、二进制/捕获源码/原源码归档 SHA-256、完整表与原用户 ID 向量 hash、禁网与回滚范围，以及逐单原结果。preparer 验证精确覆盖、owner、金额和拒绝状态；这些 hash 是来源保留材料，不是对任意 JSON 的真实性签名，仍需独立审核原函数执行过程。

生成的 `legacy-topup-settlement-facts.v1` owner 回执绑定同一 opening archive 与观察时间，严格重放只接受同一 manifest。它保存原订单引用、原额度与原应付金额、被拒绝记录和未映射用户；`runtime_settlement_available=false`。真正的 `LegacyPaymentBasis` 运行时表、商户/环境/币种绑定、旧订单验签入口、折扣消费和原子入账仍须实现，不能把事实回执当作付款或发放授权。

本次固定快照的原函数观察为 190 单中 182 单额度有效，8 单 EPay 记录被原规则拒绝。全部订单的原应付金额可读；3 单 owner 已删除。逐单观察与订单、1 份重基母计划、5 项额度配置均已在本地回执精确保留，重复导入无额外资金写入。独立业务验收与真实回调仍未执行。

### 新旧订单算术边界

preparer 额外输出 `compatibility.private.json`，只核对当前新订单的整分金额与整数 `credits_per_minor` 约束，不修改原 manifest 或冻结报价。原快照的 182 笔有效报价中，3 笔应付金额不是整分；其余整分订单中有 177 笔的原额度无法表示成整数 credits/分，仅 2 笔算术上可表示。该数字不证明渠道、验签、归属或退款就绪。

旧回调需要独立的冻结报价结算依据，保留原金额的微单位精度和原有效额度。不能通过舍入原金额、调整到账额度或重选当前汇率适配新订单表。当前报告始终为 `native_intent_replay_authorized=false`，实际运行时结算仍未开放。

### 旧 Pancake 回调验签

Core 的 [独立验签器](../services/core/src/payments/legacy_pancake.rs) 接受 owner 审阅后绑定的原订单引用、币种、微单位金额和原有效额度。它复用原始字节 RSA 验签、45 分钟重试窗口及重复 JSON 键拒绝规则；不使用新渠道的 credits/分比例或新订单金额上限重算旧报价。币种为空的旧行仍需 owner 显式绑定，不能从当前渠道配置推定迁移已经就绪。

金额保留原十进制及科学计数法语法，换算结果必须为正整数微单位，不舍入；尾随零不改变金额。与固定旧 SDK 一致，`chargedAmount` 优先于目录 `amount`，带 `listPrice` 却没有实付金额的现代报文不能证明付款。老报文允许缺少产品/商户元数据；有原产品绑定且报文携带产品键时必须匹配，原产品不会被新渠道产品覆盖。原钱包规则不校验 `lmm_merchant`，legacy 分支同样不把该字段与当前 merchant 比较；渠道归属仍需 owner 绑定。新订单的整分金额和元数据要求保持原规则。

原 Go 将 `data.orderId` 保存为供应商交易号，新 Core 则使用 `data.paymentId`。验签结果分别保留这两个标识、原 event id（缺少时回退 eventId）、原金额、原额度和载荷 hash。旧报文可以缺少 paymentId；这不代表允许绕过跨新旧订单的交易去重。后续入账必须先处理两种标识的别名与并发保护。

该函数只验证回调观察，不创建付款证明、折扣消费或账本记录，尚未接入 HTTP/RPC 的旧订单结算入口。原始事实回执仍不可授权发放。当前自测使用本地生成的 RSA 签名，不是供应商真实回调或独立验收。

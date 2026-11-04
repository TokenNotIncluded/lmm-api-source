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

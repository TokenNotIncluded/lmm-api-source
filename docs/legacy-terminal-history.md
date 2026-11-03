# 旧终态记录的离线保留

签到历史与 Hero SMS 终态订单使用 `core_migration.import_runs` 的不可变 owner 回执保留来源事实。目标必须已导入同一 archive hash、同一观察时间的身份和 opening；安装所有者再次核对原用户 ID 与个人账户映射。普通启动不导入，运行时不能写回执，Go 不读取 Core 数据库。

## 准备来源与计划

只从本机隔离恢复库导出；连接使用显式本地 Unix socket、只读源角色和原冻结快照观察时间，不能用导出当天的时间代替。文件含用户历史及密文，工作目录使用 `0700`，文件使用 `0600`，不要输出内容到终端或 Git。

| 记录 | 只读导出 | 生成 owner 计划 |
| --- | --- | --- |
| 签到 | [export_checkin_history_snapshot.sql](../scripts/export_checkin_history_snapshot.sql) | [prepare_checkin_history_snapshot.py](../scripts/prepare_checkin_history_snapshot.py) |
| Hero SMS | [export_hero_sms_history_snapshot.sql](../scripts/export_hero_sms_history_snapshot.sql) | [prepare_hero_sms_history_snapshot.py](../scripts/prepare_hero_sms_history_snapshot.py) |

导出 SQL 的 `observed_at` 是必填 psql 变量。两个 preparer 均接受 `--input`、`--output` 和 `--archive-sha256`；签到另需独立捕获的 `--expected-rows`，Hero SMS 另需 `--expected-orders` 与 `--expected-ledgers`。不能从遗漏行的导出自身推导“期望数量”。

输出为 `manifest.private.json` 和 `import.private.sql`，已有文件拒绝覆盖。审核计划后，只在明确选定的离线演练目标库中以安装所有者执行。来源绑定、身份映射、不可变回执与同来源重放在一个 SERIALIZABLE 事务内核验；不同内容重放被拒绝。所有历史奖励、扣费、退款已包含在 opening 余额中，**不能重放任何一笔资金操作**。

## 每个领域的边界

签到保留原日期、奖励整数、用户、创建时间和四项原配置。已删除用户记录保留但不重新分配给新用户。旧原始奖励单位不等于当前 credits，历史总额不做法币估值，也不授权开启每日奖励。原服务器时区、按等级随机奖励、零奖励与原生账本发奖继续单独实现及验收。

Hero SMS 保留订单及 quota ledger 的全部原字段，包括原价格字符串和原密文。只接受 completed、cancelled、failed 终态；逐订单检查预扣、退款、实际净扣、所有者及流水归属，不能让不同订单的相反错误在总额上抵消。未结束订单不能伪装成已迁移终态。密文原样保存不证明解密或页面展示可用，也不创建新供应商订单、不再次退款、不启用供应商操作。

## Hero SMS 密文恢复核对

[离线密文工具](../scripts/verify_hero_sms_payloads.py) 接受 `--manifest`、`--report` 和可选的 `--key-file`。省略密钥时只检查原 `v1:` 封装、base64url、nonce 和认证标签长度，返回 `KEY_REQUIRED`、退出码 `2`；这不是解密成功。

密钥文件必须是当前用户所有的普通文件，权限为 `0400` 或 `0600`，不接受符号链接，内容为原环境变量的有效裸值。旧版优先 `HERO_SMS_ENCRYPTION_KEY`，仅在该项未设置时回退到 `CRYPTO_SECRET`；不能拿 `SESSION_SECRET`、商店或工具市场密钥替代，也不要将密钥粘贴到聊天或放入命令行参数。

提供原密钥后，工具按固定旧版 `hero_sms.payload` 用途、SHA-256 派生和 AES-GCM 验证每个非空字段。错误密钥、错误用途、密文篡改或无法还原 UTF-8 均不算成功。报告只保存数量、错误类别和输入文件 hash，不保存明文、明文 hash 或密钥指纹，不访问供应商，也不修改数据库。使用旧版真实 Go 加密函数产生的合成向量，只能证明编解码互操作，不能代替真实旧密文恢复。

实际旧快照恢复仍须全部字段认证通过；页面权限、解密展示、供应商操作与完整业务验收继续分别证明。

## 对账与验收

[统一核对工具](../scripts/verify_legacy_cutover_rehearsal.py) 对签到逐字段比较，对 SMS 两张表使用 PostgreSQL 规范 JSON 的全部字段 hash 比较，并核验 archive、观察时间和无资金重放标记。另检查原 SMS 订单与旧流水的一致性。对账只输出数量与差异数量，不输出密文、手机号、验证码或原价格内容。

仍有资金、权利或业务缺口时报告保留 `SCOPED_PASS_WITH_GAPS`，退出码为 `2`；这不等于完整业务验收。必须分别保存导入重放与拒绝证据、逐条核对、余额和总账不变证明、密文读取、实际服务/页面、独立验收及最终停写快照的证明。旧演练回执不能当作最终切流边界。

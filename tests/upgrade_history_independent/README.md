# Upgrade History 独立验收

`independent.rs` 直接链接冻结 `liblmm_core.rlib` 调用真实 History API；
`run.py` 调用同一冻结根的实际 Core CLI，并管理两个本地 PG18 cluster、物理恢复和真实 COMMIT 回执丢失。
用例自行定义完整 Snapshot 和持久行预期，不执行作者测试。

业务判据：唯一消费者产生合成扣账后，物理恢复目标库会抹掉扣账事实，但独立 authority 的原绑定和已消费状态不变，不能再次消费。
测试还覆盖错误 authority/同物理 cluster 拒绝、writer 权限拒绝、断线重连、authority PostgreSQL 进程停启及 CLI inspect 的只读/no-open/no-retry 行为。
合成扣账和 fence/receipt fixture 不代表完整升级、真实围栏、browser、S11 或生产验收。

报告及所有运行材料在私有目录：
`/home/lightjunction/Documents/Codex/2026-10-29-upgrade-history-independent-side-a/`。
源码目录只保留本 README 和两个 runner 文件，不创建数据库、冻结源码或日志。

先执行全局 `space-first` 检查。从源码仓库根复跑，必须用新的 attempt：

```sh
XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
systemd-run --user --unit=lmm-history-independent-side-a-02 \
  --property=MemoryMax=2G --property=MemoryHigh=1536M \
  --property=MemorySwapMax=4G --property=CPUQuota=100% \
  --property=WorkingDirectory="$PWD" \
  /usr/bin/python3 -B "$PWD/tests/upgrade_history_independent/run.py" \
  --freeze-root /home/lightjunction/Documents/Codex/2026-10-29-upgrade-history-5h209odr \
  --evidence-root /home/lightjunction/Documents/Codex/2026-10-29-upgrade-history-independent-side-a \
  --attempt attempt-02
```

runner 内部取得 `/tmp/lmm-delegate/heavy-build.lock`，核验 2 GiB admission 和 cgroup 限额。
查看私有 attempt 的 `result.json`；启动命令成功不是验收通过。失败日志保留，同类失败两次停止，产品问题交产品 owner 修复，不减弱断言。
成功后清理两库、恢复副本、socket、exe、临时配置；保留冻结源码、SQL、日志、命令和哈希。无 CI、生产、DMIT、付费调用或真实账号。

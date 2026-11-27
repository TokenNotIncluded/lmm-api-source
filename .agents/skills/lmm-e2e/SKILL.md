---
name: lmm-e2e
description: "用户要求在隔离环境执行 LMM API 业务端到端测试或独立验收时使用。按用户旅程选定真实入口、角色、数据和最终状态，区分浏览器、进程、数据库及模拟外部服务覆盖。"
---

# 在隔离环境验收 LMM API 用户旅程

先把验收条件写成一句可观察的行为。例如，管理员关掉公开导航中的 AI 目录后，匿名访客刷新页面应看不到该链接，Core 重启后设置仍然有效。明确本轮的角色、起点、操作、最终读回方式，再选择能覆盖这条路径的 runner。独立验收由业务修复者以外的执行者完成。记录冻结版本，保留失败现场。

## 选入口

以下相对路径以源码仓库根目录为起点。先读相应入口及其参数；历史报告只对应当时的输入。

| 要回答的问题 | 入口与数据 | 能交付的证据 |
| --- | --- | --- |
| 用户能否在浏览器保存设置、刷新和重启后读回？普通用户是否被拒绝？ | [控制台浏览器 runner](../../../tests/console/browser_acceptance/run.py) 调用 [Chromium 用例](../../../tests/console/browser_acceptance/browser.mjs)。使用实际 Web、Core、私有 PostgreSQL 和合成账号。 | 按用例报告登录、Notice/公告、公开导航、故障保存、权限等。订阅计划和兑换活动的创建、重启读回与普通用户拒绝也有具名用例，以实际执行的 cases 为准。 |
| API 写入、回滚或提交回执丢失时，数据库里究竟留下了什么？ | [控制台进程验收](../../../tests/console/independent_process/README.md) 使用实际 Core、TCP 登录、受限 runtime 及独立 SQL 读回。 | 按请求与审计对账，包括并发写入和提交不明；页面体验另由浏览器用例验证。 |
| Core 入账后、Go 写回执前两进程退出，重启是否重复入账？ | [支付恢复](../../../tests/payments/README.md) 使用绑定摘要的预构建产物、自建 PostgreSQL 和合成支付事件。 | 核对原 job key、receipt、唯一 journal 及借贷两腿。报告所选恢复阶段，真实渠道收付款另列未测。 |
| 问题是否位于客户端传输或外部协议夹具？ | [A10 客户端](../../../tests/e2e/a10/README.md) 使用合成服务；[Mocklab](../../../tests/mocklab/README.md) 提供可控上游。 | 即使 A10 使用 Chromium，身份后端仍是合成的。Mocklab 的组件报告用来定位故障；应用的授权、选路与扣费需接实际 Core、隔离库并核对最终状态。 |

若请求涉及其他页面、模型协议、计费或多组件流程，按[质量指南](../../../docs/quality.md)在候选源码中找对应入口。记录每个实际经过的组件和替身的位置。缺少应用驱动时，列出缺失的那一段，继续执行已经接通且在范围内的用例。用户体验反馈及 issue 归纳可复用 `.agents/skills/test-feedback`，新增回归的显式任务可复用 [create-verification-skill](../create-verification-skill/SKILL.md)。

## 执行一条真实浏览器路径

以下示例在项目开发机上运行控制台**新夹具**。从已绑定候选的源码仓库根目录执行。将 `LMM_E2E_COMMIT` 设为本轮指定的完整提交，`LMM_E2E_CORE` 设为该提交对应的 Core 二进制绝对路径，`LMM_E2E_CORE_SHA256` 取自独立构建记录，`LMM_E2E_PLAYWRIGHT` 指向已安装 Playwright 的 `index.mjs`。保存构建出处；摘要可检查二进制漂移。新夹具会在私有目录运行一次 Bun 锁定安装并构建 Web，再初始化临时库和本地账号。先确认 Bun 1.3.14、Chromium、PostgreSQL 工具及 user systemd 已就绪，当前资源允许此构建。

```bash
set -eu
repo=$(git rev-parse --show-toplevel)
commit=${LMM_E2E_COMMIT:?需要候选完整提交}
core=${LMM_E2E_CORE:?需要候选 Core 绝对路径}
digest=${LMM_E2E_CORE_SHA256:?需要构建记录中的 SHA-256}
playwright=${LMM_E2E_PLAYWRIGHT:?需要 Playwright 模块绝对路径}
test "$(git -C "$repo" rev-parse HEAD)" = "$commit"
git -C "$repo" diff --exit-code "$commit" -- tests/console/browser_acceptance/
test -z "$(git -C "$repo" ls-files --others --exclude-standard -- tests/console/browser_acceptance/)"
run_id=$(python3 -c 'import uuid; print(uuid.uuid4().hex[:12])')
root="/home/lightjunction/Documents/Codex/e$run_id"
systemd-run --user --wait --pipe --collect \
  -p MemoryMax=2G -p MemoryHigh=1536M -p MemorySwapMax=4G \
  -p CPUQuota=100% -p TasksMax=256 -p LimitCORE=0 \
  python3 -B "$repo/tests/console/browser_acceptance/run.py" \
    --repo "$repo" --commit "$commit" --core "$core" --core-sha256 "$digest" \
    --root "$root" --playwright "$playwright"
```

runner 从指定提交归档 Web 源码。工作区未提交的产品修复不在新夹具里。它会创建新的私有输出目录，要求其父目录为上例的 `Codex`，`root/tmp` 路径至多 55 字节。它取得共享重型构建锁后检查可用内存至少 2 GiB 并核对 cgroup 限额。已有候选夹具的复验按[复用条件](../../../tests/console/browser_acceptance/README.md)操作，确认原 PG 已干净关闭、冻结输入匹配并使用新的输出目录。`--diagnose-selector` 产生定位证据，`--ordinary-only` 仅覆盖普通用户组，二者都依赖 `--reuse-root`。

## 看业务结果

- **保存和跨角色读回。** 以导航为例，让管理员切换开关并保存，核对 PUT 及返回值，再查选项、刷新页面。另开匿名上下文核对实际公开链接，在桌面和两个窄屏视口操作菜单。Core 重启后核对 PID 和设置。以普通账号直接打开管理深链并尝试保存，核对 403、`SYSTEM_OPTIONS_FORBIDDEN` 和拒绝前后的公开值。
- **失败后的最终状态。** Notice 的存储错误用例撤销私有夹具的审计写权限，检查错误提示、3.5 秒内仅一次写入及数据库快照。提交不明用例丢弃真实 PG 的 COMMIT 回执，核对 `SYSTEM_OPTIONS_COMMIT_UNKNOWN`、故障证据和写次数，然后读取并刷新确认这一次实际提交的值。其他运行没有读回时，将最终状态保留为未知。
- **复现与报告。** 从 `commands.json`、`results.json` 和 `browser-result.json` 对到具体用例、子进程退出码和证据。一个用例一行写清「候选/角色、输入和动作、预期、实测、状态、脱敏证据位置」。DOM、截图及请求记录与业务读回配对。区分本轮通过、复用旧证据、断言失败、前置受阻和未运行，保留原始状态。失败后停止自建进程，保留私有现场；修复后换新目录，用原反例复验。报告中隐藏 cookie、密钥、密码、连接串和真实用户数据。
- **授权范围。** 使用自有本地夹具，核实数据库和服务归属。生产脚本 `apps/web/scripts/production-acceptance.mjs verify` 会创建用户和 API key、授予额度并调用真实上游，排除在本地执行之外。交付时注明执行者、业务路径、结果和仍缺的证据；技能内容或历史 PASS 本身没有本轮运行结果。

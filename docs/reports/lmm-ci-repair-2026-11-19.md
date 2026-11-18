# LMM API 主线 CI 阻塞修复

日期：2026-11-19（Asia/Taipei）。

## 完成结果

本轮修复了阻断主线自动检查的 Web 格式、版权声明位置和 Core Clippy 问题。Web 格式、lint、类型与版权检查，以及 Core 全目标 Clippy 均已通过开发机自测。**自测通过，独立业务验收未执行；这份记录不表示整站 CI 或生产验收已完成。**

第一批 62 个 Web 格式修复已提交为 [7ebaf642](https://github.com/TokenNotIncluded/lmm-api-source/commit/7ebaf6422742beb7fcfdd874c2a25c916b0cf00b)。Core 修复、版权声明位置调整和本报告随同一后续提交交付。并行提交的 [HTTP 路由验证报告](https://github.com/TokenNotIncluded/lmm-api-source/commit/b76a68bc8d66f15da829458cb44e9a6afbf8b743)已保留。

## 问题与改动

### Web 格式与版权检查

原 [CI 44368105622](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44368105622) 在 Web 格式检查阶段失败，后续检查没有执行。使用仓库现有的受保护版权头 formatter 修正了 62 个文件，未修改格式规则、依赖或工作流门槛。

差异检查确认，60 个文件在忽略源码位置、注释位置与 JSX 排版空白后，语法结构一致。另外两项分别为等价的 JSX 空格表达和测试导入重排，已检查实际差异。对应文件是 `assistant-setup-tool.tsx` 与 `auth-session.test.ts`。

第一批推送后的 [CI 44374854572](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44374854572) 已通过格式检查，随后报告 `native-users/page.test.tsx` 的版权声明不在文件头。将原有 LIghtJUNction／AGPL 声明移到第一行后，全 Web 版权检查通过；Bun 的类型检查指令仍紧邻其导入语句。

### Core 编译检查

`IdentityStore::api_key_routing_groups` 原先嵌套的两个条件触发 `collapsible_if`。将它们合并为短路逻辑后，Auto 分组仍跳过显式分组查询，非 Auto 分组仍在授权不通过时返回 Forbidden；校验顺序与拒绝条件保持一致。

随后，全目标 Clippy 检查发现锁顺序测试只使用共用 abuse 夹具中的部分函数。参照仓库现有做法，在该测试目标的共用模块导入处标记允许未使用辅助函数，保留其他测试对这些函数的调用。注解仅作用于这个测试模块，未关闭全局 lint，也未修改测试断言。

## 开发机验证

验证通过 ssmcp 在既有开发工作区运行，使用锁定依赖和既有 Rust 构建缓存。Core 检查采用 Rust 1.100.0、两个构建任务、关闭 debug 信息与增量编译；本轮没有初始化数据库或执行真实付费供应商调用。

| 检查 | 实际结果 | 证据 |
| --- | --- | --- |
| 全 Web `format:check` | exit 0；62 项格式问题已修正 | 开发机记录，GitHub 后续 CI 同阶段也通过 |
| 全 Web `lint` | exit 0；0 errors，350 warnings | 既有 warnings 保留 |
| 全 Web `typecheck` | exit 0 | 13.192 秒 |
| 全 Web `copyright:check` | exit 0；checked 2592，added 0，updated 0 | 版权位置调整后检查 |
| 版权文件格式 | exit 0 | 仅检查受影响的测试文件 |
| Core `cargo fmt --all --check` | exit 0 | 业务代码条件合并后检查 |
| 新增测试注解的格式 | exit 0 | 定向 rustfmt 检查 |
| Core `cargo clippy --locked --offline --all-targets -- -D warnings` | exit 0 | 最终执行 234.85 秒 |

Clippy 的首次开发机检查退出码为 101，报告两项共用测试辅助函数未使用；修复后的最终执行退出码为 0。初始 CI 失败、后续版权失败和首次 Clippy 失败均保留在[结构化证据](lmm-ci-repair-2026-11-19.evidence.json)，不将中间失败改写为通过。

本轮未新增测试用例。通过的检查没有在输入未变化时重复执行；格式检查、lint 和类型检查不计作业务测试数量。最终 Clippy 于 2026-11-19 03:02:54（Asia/Taipei）完成。

### 主要复现命令

```bash
mise exec -- bun run --filter @lmm/web format:check
mise exec -- bun run --filter @lmm/web lint
mise exec -- bun run --filter @lmm/web typecheck
mise exec -- bun run --filter @lmm/web copyright:check
cargo fmt --manifest-path services/core/Cargo.toml --all --check
cargo clippy --manifest-path services/core/Cargo.toml --locked --offline --all-targets -- -D warnings
python3 -B scripts/check-docs-brand.py --docs-only
```

每项检查的实际命令、耗时、退出码与受影响文件摘要见结构化证据。修改测试模块后仅补查该文件格式，未重复执行已通过的全 Core 格式检查。

## 当前边界与下一步

Web 仍有 350 条 lint warnings，但当前 lint 命令退出码为 0。完整 Web 测试、生产构建和真实数据库业务回归未在本轮开发机执行；后续自动化结果应按 [GitHub CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/workflows/ci.yml) 的实际状态继续记录，不能由这些静态检查推定为通过。

本轮没有部署、操作生产数据库或调用真实付费供应商。渠道、Key 与四协议的既有运行证据继续使用[上一轮报告](lmm-channel-key-protocol-2026-11-19.md)，本轮不重复累加其测试数量。下一步是读取本批提交的 CI 后续阶段，处理其实际失败，并继续既定业务验收。

## 交付与自检

报告和结构化证据与源码一起保存在 GitHub；副本归档到 [Google 云盘](https://drive.google.com/drive/folders/10XD834rnPHw_fMjZOfebUIQmQ5t1Ri73)。全量文件摘要共覆盖 65 个本轮修改的源码／测试文件。

| 自检项 | 结论 |
| --- | --- |
| 价值 | 修复实际阻断主线检查的问题 |
| 清晰度 | 分开说明源码修复、本地自测和 GitHub CI 状态 |
| 验证强度 | 直接运行原格式、版权与编译检查，保留失败及最终退出码 |
| 覆盖范围 | 明示未运行的业务回归、独立验收和部署 |
| 实现合理性 | 使用现有 formatter 与测试夹具惯例，条件合并保持授权语义 |

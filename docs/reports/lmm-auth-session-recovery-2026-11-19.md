# LMM API 浏览器会话恢复实现报告

日期：2026-11-19（Asia/Taipei）。证据同时保留 UTC 发布时间。  
源码仓库：`TokenNotIncluded/lmm-api-source`，分支：`main`。  
当前认证修复：[300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86](https://github.com/TokenNotIncluded/lmm-api-source/commit/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86)。  
结构化证据：[lmm-auth-session-recovery-2026-11-19.evidence.json](./lmm-auth-session-recovery-2026-11-19.evidence.json)。

## 结论与范围

本轮修复了恢复后丢失目标页面，以及恢复入口反复卡住的两类问题，两批均已发布到 `arch-shadow`。更新后的 native 渠道页载入新 Web 脚本，未显示恢复提示，两个渠道相关读取接口返回 HTTP 200。本轮开发自测和上线后的只读观察已完成；完整管理员业务验收及资金验收未运行。

修复前的现场观测显示，native 登录页同时出现恢复提示和普通密码表单；恢复请求返回 HTTP 200，协议分类为 `no_cookie`。本报告未收录现场 Cookie、token 或账号资料。最早一次登录中断为何发生，现有证据仍不能确定；下面的修复针对已经证明会让用户持续卡住的机制。

## 已证明的机制

Cookie 防护状态（fence）表示当前页面不能继续启动可能写入登录 Cookie 的操作。它会保留“本页此前出现不确定结果”这一事实，不能仅靠删除本地记录就认定服务器已退出或旧请求已失效。

| 机制 | 因果关系 | 证据与修复 |
| --- | --- | --- |
| 被阻断的密码提交破坏恢复记录 | 普通密码表单仍可提交；`login()` 在检查 fence 之前改变登录 intent，使已有恢复交接记录失效。Cookie 锁随后拒绝请求，用户既没有登录成功，也失去了原恢复入口。 | 新增 API 红测复现 intent 被改变；修复后在改 intent 之前同步检查。表单订阅 fence，并在密码和 Passkey handler 入口实时检查。 |
| 失效交接记录被反复使用 | 交接记录的 intent 或共享 marker 已变化，原“重新登录”入口仍试图消费旧记录。错误状态没有切换到可重新建立交接的分支。 | 交接有效性同时检查 marker；失效时重新执行显式恢复，进入新文档建立交接，错误统一回到既有状态分类。真实 Chromium 场景覆盖该路径。 |
| 另一标签恢复后，旧标签没有出口 | 另一标签清除共享 marker 后，旧标签内存中的 fence 仍应保留；原入口因找不到 marker 而失败。 | 仅在本页仍为 uncertain、共享 marker 严格不存在、且本地操作已空闲时，允许显式替换整个文档。旧文档不会因此解除 fence；pending writer 仍被拒绝。 |

上述实现对应固定版本的 [Cookie 协调器](https://github.com/TokenNotIncluded/lmm-api-source/blob/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86/apps/web/src/lib/auth-cookie.ts)、[恢复协调逻辑](https://github.com/TokenNotIncluded/lmm-api-source/blob/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86/apps/web/src/lib/auth-cookie-reconciliation.ts)、[恢复组件](https://github.com/TokenNotIncluded/lmm-api-source/blob/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86/apps/web/src/components/auth-cookie-recovery.tsx)和[密码 API](https://github.com/TokenNotIncluded/lmm-api-source/blob/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86/apps/web/src/features/auth/api.ts)。

## 两批实现

第一批 [8abbbdd1](https://github.com/TokenNotIncluded/lmm-api-source/commit/8abbbdd141a221d781279404c154298f7acb95e6) 解决目的地丢失，共修改 3 个 Web 文件，新增 67 行、删除 17 行。文档交接、成功恢复、显式续期和重新登录均保留原目标，复用已有 `sanitizeAuthRedirect`。测试同时覆盖查询参数和 hash；客户端导航允许路由器按既有规则规范化重复参数，整页交接仍检查原 URL。

第二批 [300d41c2](https://github.com/TokenNotIncluded/lmm-api-source/commit/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86) 修复上表中的循环，共修改 8 个 Web 源码、测试及浏览器脚本文件，新增 199 行、删除 13 行。密码、Passkey、OAuth、HumanGrant 和已打开的 WeChat 确认入口受现有 fence 约束；没有新增界面文案。正常密码登录仍在等待 Cookie 锁之前取消旧 OAuth，内部锁的可用性检查仍在锁内。

这两批修改没有把“结束本标签页”改写为“服务器已退出”，也没有放宽权限判断、待完成 Cookie 写入的串行约束或恢复所需的服务器证明。源码和回归入口均以提交 SHA 固定，见结构化证据中的文件对象与 SHA-256。

## 开发自测与保留失败

浏览器回归使用真实 Chromium、隔离浏览器上下文和本地合成服务，运行的是项目实际恢复组件与请求代码。这些结果证明已覆盖场景下的前端行为，不代表真实管理员登录、渠道管理或付费请求已经验收。

| 第二批检查 | 结果 | 证据 |
| --- | --- | --- |
| 真实 Chromium 恢复用例 | 18 通过；32.247 秒；exit 0 | `scratch/auth-recovery-return-20261118/browser-loop-v2.log`、同名 `.json` |
| 完整 Web typecheck | 14.256 秒；exit 0 | `scratch/auth-recovery-return-20261118/typecheck-loop-v1.json` |
| 恢复组件局部退出 | 2 通过 | `scratch/auth-recovery-return-20261118/local-recovery-loop-v1.log` |
| 本地恢复及旧页面权限边界 | 6 通过 | `scratch/auth-recovery-return-20261118/local-authority-loop-v1.log` |
| Cookie API 回归 | 8 通过；Bun 记录 0.314 秒 | `scratch/login-fence-selftest/auth-cookie-green.log` |
| 登录表单回归 | 48 通过；Bun 记录 12.90 秒 | `scratch/login-fence-selftest/user-auth-form-green.log` |
| 恢复相关 4 文件 lint | 0 warnings、0 errors | `scratch/auth-recovery-return-20261118/recovery-lint-v4.json` |
| API、表单及对应测试 4 文件 lint / 格式 | lint 0 warnings、0 errors；定向格式化和 diff 检查通过 | 实施会话原始工具输出，命令及范围写入结构化证据 |

浏览器完整结果在开发机 `/tmp/lmm-auth-reconciliation-kEjOtL/result.json`。其 5 个源码哈希逐一匹配提交 `300d41c2`，本报告的结构化证据保存这些哈希、场景名称及结果文件哈希，没有复制请求体。第一批另有 16 个 Chromium 场景通过（28.110 秒）、完整 Web typecheck 通过（29.056 秒）及 2 个局部恢复测试通过，记录在同一 scratch 目录。

以下失败保留原日志，没有用通过结果覆盖：

| 失败记录 | 失败原因 | 后续处理 |
| --- | --- | --- |
| `browser-v1.log`，exit 1 | 测试精确匹配重复 query 的原始编码；TanStack 将重复 `tag` 规范化为 JSON 数组，导致 URL matcher 超时。 | 固定断言规范化后的 URL，仍要求保留两个参数值与 hash；文档交接继续断言原 URL。`browser-v2` 通过。 |
| `browser-loop-v1.log`，exit 1 | 测试夹具在 `owner.reload()` 之后仍期待旧文档中的合成内存身份。 | 将身份检查移到 reload 前，保留检查；新文档场景继续验证旧标签出口。`browser-loop-v2` 通过。 |
| `password-preflight-red.log`，exit 1 | 新增回归证明被阻断的密码调用仍改变 login intent。 | 在 intent 变化前加入同步检查；定向复验同时断言 intent 不变、HTTP 请求不增加。 |

## 发布记录与线上只读观察

| 批次 | 范围与固定源码 | 发布状态 |
| --- | --- | --- |
| 回跳修复 | Web only，`target=arch-shadow`；`8abbbdd1` | [运行 44340342536](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44340342536) 成功。部署 job `134932889337` 于 `2026-11-18T15:59:23Z` 完成（台北时间 23:59:23）；日志确认 served HTML 匹配、health 通过，`arch-shadow` 汇总 `ok=28 changed=7 failed=0`。 |
| 循环修复 | Web only，`target=arch-shadow`；`300d41c2` | [运行 44343809950](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44343809950) 成功。部署 job `134942165546` 于 `2026-11-18T16:12:53Z` 完成（台北时间 2026-11-19 00:12:53）；日志确认 served HTML 与发布 artifact 匹配、health 通过，`arch-shadow` 汇总 `ok=28 changed=7 unreachable=0 failed=0 rescued=0 ignored=0`。 |

用户已自行完成登录并进入 `/channels`。主任务随后整页刷新，观察到脚本 `index.d2eff6ef66.js`、空的 alert 列表，以及两个读取接口的 HTTP 200 响应。以下收据位于原浏览器 bridge 的 receipts 目录，完整路径与 SHA-256 写入结构化证据。

| 上线后只读观察 | 收据 ID |
| --- | --- |
| `/channels` 刷新后载入新脚本，alert 列表为空 | `lmm-auth-loop-20261119-dom-after-refresh` |
| `/api/channel/ops` 返回 HTTP 200 | `lmm-auth-loop-20261119-channel-ops` |
| `/core/v1/admin/routing/configuration` 返回 HTTP 200 | `lmm-auth-loop-20261119-native-channel-requests` |

这些观测说明，用户已有会话下的新页面能够访问渠道读取接口，且观察时没有恢复提示。登录由用户自行完成；本轮自动化没有提交登录凭据，也没有写入渠道。完整登录提交流程、渠道写入流程和资金流程未作验收。

## 证据边界与成文检查

| 检查维度 | 结论 |
| --- | --- |
| 解决的问题 | 回跳丢失与可复现的恢复死路有固定源码和回归证据；最初中断触发仍未知。 |
| 表述与术语 | 区分本地 fence、文档交接、服务器证明与部署结果，未把本地状态清理称为服务器退出。 |
| 自测强度 | 保留真实 Chromium 行为、API 红测及通过结果；合成服务测试、修复前现场分类和上线后只读观察分别记录。 |
| 验证完整性 | 两批发布与上线后只读访问已确认；工具未执行登录提交或渠道写入，完整管理员业务和资金验收未运行。 |
| 设计边界 | 显式换文档提供出口，旧文档仍被阻断；现有权限、Cookie 锁和证明条件保持。 |

写作采用用户指定的 [Research Paper Writing 技能](https://github.com/Master-cai/Research-Paper-Writing-Skills/blob/77e7c2c1ba06f7d71844873147665437a03aac1b/research-paper-writing/SKILL.md)的段落组织与结论—证据对齐方法，并遵循项目 [docs-maintenance](../../.agents/skills/docs-maintenance/SKILL.md)。主任务运行 `python3 -B scripts/check-docs-brand.py --docs-only`，exit 0；正文与 JSON 另行回读。成文检查没有另起应用测试或业务验收。

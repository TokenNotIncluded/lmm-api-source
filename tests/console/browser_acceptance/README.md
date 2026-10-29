# Notice / 公告真实浏览器独立验收

2026-10-30 最终状态：**本侧线 8/8 约定组 PASS**，不是完整项目/发布/部署结论。
最新报告：`/home/lightjunction/Documents/Codex/cb1030a10/REPORT.md`。
a10 冻结 HEAD7c3ebb8 + 主线程两行 Notice 修复，Bun1.3.14 复用现有依赖只构建一次。
本轮实跑管理员登录、Notice保存刷新、503不重试、真实COMMIT UNKNOWN四组；公告、
其它设置只读、窄屏、普通用户无写入口四组按用户允许的单组件变更边界 REUSED_PASS。
完整Web typecheck、该组件lint/format独立计为开发检查，均PASS。
原 a8 产品异常FAIL和a4/a6定位FAIL均保留；新输入复验不改写历史失败。

原 a6 失败选择器已在 a7 实际 DOM 中确认命中收起助手的 textarea（不是设置字段）：
`name=message`，form=`assistant-prompt-form`，位于 `.settings-sheet` 外；祖先 rail
为 inert/aria-hidden/opacity=0/width=0。业务断言改用唯一 `.settings-sheet`，仍要求
没有 enabled 输入/选择/可编辑/切换控件，所有 Save Settings disabled，写请求数不增加。
原 a4/a6 两次定位失败、a6 十张截图和 DB 未改写。

本目录只有 runner 和说明。所有用例记录、截图、构建、依赖、数据库、凭据与冻结输入
都写入新的 `/home/lightjunction/Documents/Codex/` 私有直属目录。没有产品修改、CI、
生产访问、DMIT、付费上游、成功响应 mock 或调试 persona。

`run.py` 固定当前 HEAD（默认 `7c3ebb86cf406c5f0a268ea2eb90edb985271751`）的
`git archive`，排除工作树修改。初始新装模式会在冻结源码中构建 Web，记录原始文件、构建输出及
构建后生成文件的哈希。缺少依赖先记 `dependency-preflight.json`，只允许一次
240 秒、有 frozen lockfile、禁 lifecycle scripts、单网络并发的安装；构建限时 600 秒。
复用机器已有 Playwright/Chromium，不下载浏览器或添加仓库依赖。

Core 固定用户交付的实际 main，SHA-256：
`6eca9a6e8b270bc7368057469dbd8185ad287db74589df3f3a223f610a53ec31`。
其交付目录 `delivery-native-binding.json` 将其绑定到 `7c3ebb86`；此说明不把 dev
产物升级为 release-profile/发布/部署证据。

当前续验使用以下模式，不执行 install/build/init-db/create-first-owner/register：

```bash
XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus \
systemd-run --user --wait --pipe --collect --unit=lmm-console-browser-a08 \
  -p MemoryMax=2G -p MemoryHigh=1536M -p MemorySwapMax=4G \
  -p CPUQuota=100% -p TasksMax=256 -p LimitCORE=0 \
  python3 -B /home/lightjunction/Documents/GITHUB/lmm-api/generated/lmm-api-source/tests/console/browser_acceptance/run.py \
  --repo /home/lightjunction/Documents/GITHUB/lmm-api/generated/lmm-api-source \
  --core /home/lightjunction/Documents/Codex/2026-10-29-upgrade-history-5h209odr/lmm-core \
  --root /home/lightjunction/Documents/Codex/cb1030a8 \
  --playwright /home/lightjunction/.npm/_npx/31e32ef8478fbf80/node_modules/playwright/index.mjs \
  --reuse-root /home/lightjunction/Documents/Codex/cb1030a6
```

上面是实际 a8 命令；Root 已存在，不可覆盖。下一次经授权复验须使用新 Root/unit。
a7 使用同样命令但 Root/unit 为 a7，并添加 `--diagnose-selector`，只记录原选择器
命中 outerHTML/form/祖先/样式，不将业务用例标 PASS。systemd 服务必须传绝对脚本路径。
`--reuse-root` 逐文件验证原 source/dist、Core SHA，将已停止且正常关闭的 PG 写时复制
到新 Root 后启动，仍用旧夹具账号真实网页登录；不会写原失败 DB，不调整注册边界。
Notice/公告只有原结果 PASS 才可标 `REUSED_PASS`，最终矩阵保留此区别。


先按全局 space-first 检查空间、窄范围 dust 和同前缀 residue；不清理旧失败材料。
Root 须简短（其 `tmp` 路径最多 55 字节），避免 Chromium 的 Unix socket 路径超限。
runner 持有 `/tmp/lmm-delegate/heavy-build.lock` 串行执行所有重型工作，取锁后要求
MemAvailable ≥ 2 GiB，并检查 cgroup 上限；锁等待最多 30 分钟，每 15 秒输出状态。
Chromium 仅创建本轮独立上下文，限制 renderer 数量；整个进程树受上述 cgroup 限制。

数据库夹具复用冻结 HEAD 中 `independent_process/run.py` 的新装流程及 `pg_fault.py`，
只将临时目录重定向到私有 Root，并把 Web origin 指向实际静态服务器。使用真实 CLI
`init-db`、`create-first-owner`、正式 grant、非 owner runtime、真实网页密码登录。
不运行原 API 用例矩阵；其 fixture-evidence 中 NOT_RUN 不计作本轮 API 验收通过。
静态服务器原样代理真实 API，fixture 控制端仅供 Node runner 使用随机密钥调用；
浏览器不向外部网站发请求，不植入登录 token。

浏览器顺序执行：管理员登录/身份；Notice 编辑保存与刷新；公告新增、保存、两别名
一致与刷新；其它设置只读；390×844 操作；真实审计 INSERT 撤销触发保存失败；真实
PostgreSQL COMMIT 回执丢失触发 UNKNOWN；普通用户深链接无写入口。失败即停，其后
NOT_RUN。故障保存等待 3.5 秒计数写请求，禁止自动重试；同时核对真实数据库状态。
截图、可访问树、请求结构及最终矩阵独立保存，不以截图或 API 200 单独判通过。

遇产品失败不自行修产品，不弱化断言。夹具错误修正也须保留前轮证据并开新 Root；
同类失败两次停止。成功停止全部自建进程、删除本轮数据库/凭据/冻结工作目录/依赖/
构建产物，保留报告、截图、runner 与哈希；失败停止进程并保留完整原件。

边界：不代表完整项目验收；存量升级/围栏、多模型协议、支付、部署、生产和 S11
资源接受度均未覆盖。实际执行结论以私有 Root 的报告为准。

a8 当时只记录了 pageerror 消息、可访问 DOM 和断言调用栈，没有产品原始JS栈。
后续runner增加 pageerror.name/stack 和完整.dom.html，已在a9/a10实跑采集流程；
a10 pageerror0，无新的异常栈。不忽略预期503触发的未处理页面异常。

修复复验使用上面 a8 命令的基本参数，但 Root/unit 改为 a10，并追加：

```text
--notice-overlay /home/lightjunction/Documents/GITHUB/lmm-api/generated/lmm-api-source/apps/web/src/features/system-settings/maintenance/notice-section.tsx
--bun /home/lightjunction/.local/share/mise/installs/bun/1.3.14/bin/bun
```

完整实际命令已保存至 a10/invocation.sh；新复验必须使用新 Root。
该模式验证组件必须恰好从 async/mutateAsync 改为同步/mutate，冻结其完整副本与哈希，
其它文件只来自git archive7c3ebb8；依赖从a6写时复制，无安装、无共享dist写入。
固定Bun缺失/不匹配或现有依赖不兼容则失败，不换版本重建。构建只调用一次。
完整typecheck和单文件lint/format在同一冻结Web串行执行，独立保存开发检查结果。
Notice成功值刻意不同于夹具已有值，保证真实发出写请求；写次数从request事件计数，
不依赖仅统计已完成响应。503/UNKNOWN均校验对应错误toast、3.5秒仅1次写和pageerror0。
UNKNOWN必须有真实PG提交回执丢失证据，并实际读取和刷新对账。

`--ordinary-only` 用于独立普通用户组，不执行设置保存或注册；a9已实跑通过。
所有新轮继续用已停止a6 PG的写时复制副本和原账号，不改普通注册403边界。

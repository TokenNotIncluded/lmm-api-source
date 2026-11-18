# LMM API 分组请求限额编辑 — 2026-11-18

## 交付结果

管理页的分组请求限额已改为逐行表单。管理员可以添加分组、填写每个固定时间窗口的请求次数、修改已有配置或移除某一组的限额，再一次保存整份策略。新增行自动获得输入焦点，手机布局将组名与次数分行排列。

源码提交：[c1890bc673c3751c992fb536a2180959b756a004](https://github.com/TokenNotIncluded/lmm-api-source/commit/c1890bc673c3751c992fb536a2180959b756a004)。

## 行为与边界

- 每个分组填写精确组名和 1–100000000 的整数次数，最多 1024 项。保留 Unicode、大小写与未列入其他目录的有效组名；不会自动修剪或规范化名称。
- 重复组名、空组名、含空格的非法组名、空次数、零、小数和超出上限的次数在保存前被拒绝。重复名称不会覆盖另一行。
- 删除一行取消该组的分组限额；用户维度限额继续独立生效。所有新增、编辑和删除都先保留为草稿，点击保存后才提交。
- 保存继续携带当前用户、会话和 expected_revision。冲突后保留草稿，不自动重试写入；显式重新读取会采用服务器当前配置与版本号。
- 保存与重新读取期间禁止继续编辑。成功保存后清除未保存状态。
- 新增 8 个文案键，覆盖 en、zh-CN、zh-TW、fr、ru、ja、vi；保留原有“分组名称”等共享译文。

该改动复用现有请求限速接口与校验合同。次数按逻辑请求计数，与 Token 数量和金额无关；同一用户的多个 API Key 共享用户限额，同一组的所有用户共享分组限额。Core 计数、权限、资金、数据库结构和路由执行代码未改动。

[完整请求限速说明](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/request-limits.md) · [当前项目进度](https://github.com/TokenNotIncluded/lmm-api-source/blob/main/docs/progress.md)

## 开发自测

| 检查 | 结果 | 覆盖范围 |
| --- | --- | --- |
| native-settings.test.tsx | 7 通过，0 失败 | 策略语义、版本保存、冲突与重新读取、组名精确保留、逐行增删改、非法和重复输入、移除最后一组、既有 Auto 行为 |
| 修改文件格式 | 通过 | 两个 TypeScript 文件，保留许可证头 |
| 修改文件 lint | 0 警告，0 错误 | 两个 TypeScript 文件 |
| Web 完整类型检查 | exit 0 | 当前工作目录的完整 Web 类型检查 |
| Web 生产构建与 bundle 检查 | exit 0 | just build-web，包括既有首屏包体检查 |
| 七语言同步 | exit 0 | 7 个 locale 各新增相同的 8 个键，插值保持一致；同步报告和 extras 无额外差异 |
| Chromium 实际交互 | 通过 | 新行焦点、重复提示、精确请求体、保存期禁用、冲突保留、显式重新读取 |
| 4 个浏览器布局 | 全部通过并实际看图 | 1440px 英文浅色、1440px 中文深色、390px 中文浅色、320px 法语深色；均无横向溢出、控件裁切或页面运行错误 |

定向测试从 apps/web 运行：

```sh
mise exec -- bun test --preload ./scripts/test-preload.mjs --timeout 15000 ./src/features/system-settings/request-limits/__tests__/native-settings.test.tsx
mise exec -- bun run oxlint -c .oxlintrc.json src/features/system-settings/request-limits/rate-limit-section.tsx src/features/system-settings/request-limits/__tests__/native-settings.test.tsx
```

类型检查和构建从源码仓库根目录运行：

```sh
mise exec -- bun run typecheck:web
mise exec -- just build-web
```

浏览器检查渲染真实的 RequestLimitEditor 与项目样式，通过本地合成接口观察请求体和状态；外部网络被拦截。它验证前端行为与布局，未运行真实 Core 联合测试或生产账号操作。独立业务验收未执行。

## 运行记录

开发机父仓库的 out/lmm-request-limits-20261118/rate-limit-editor-c1890bc6/ 保存定向测试、lint、类型检查、构建、浏览器结果及临时预览材料。初次直接调用全站格式包装脚本退出 1，无输出。随后通过已安装的 Bun 命令按两个目标文件执行格式化，退出 0，许可证头保持原样。原失败日志保留。

本轮修改基于既有 merge-source 工作目录。并行合入的渠道与网络访问报告保持完整；原开发主目录的未提交工作未纳入本轮提交。未触发新的远程 CI、发布流程或生产部署。

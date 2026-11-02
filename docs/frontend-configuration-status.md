# 首页配置修复：已验证范围与后续工作

本批次修复首页与公共导航的配置读取，不代表整个前端或业务系统已经验收完成。

## 已实现

- 保留显式配置的站点名称和 Logo，包括历史名称和图片路径；空配置仍使用默认值，图片加载失败仍有回退。
- 公共页与工作台共用导航策略。新增七个入口开关，保留旧配置默认行为、已有登录限制和未知扩展字段。全部关闭时不恢复默认链接，首页重复入口同步隐藏。
- 首页展示和复制使用同一份 API 示例。共用地址解析器，支持自定义域名、端口和路径前缀；无效地址不生成可复制命令，示例 URL 正确处理 shell 引号。
- 目录手势提示接入现有七种语言，静止时隐藏无意义的百分比，实际滚动时仍显示进度。
- 移除重复语言字典和公共布局循环导入，长地址允许换行。

## 本地验证

13 个相关测试文件的最终结果共 87 项通过：品牌显示与图片回退、导航策略、首页实际组件、目录手势与多语言渲染、API 地址与复制示例、移动页头、模型图中的入口。各文件独立运行，避免 DOM 测试环境相互污染；修复前失败日志保留，不计入通过结果。

完整 Web 类型检查、本次修改的 lint/格式检查、生产构建及 bundle 检查通过。最终构建记录覆盖 2,752 个输入文件和 598 个产物文件，构建前后输入一致。冻结记录 SHA-256：`64c4f29e263442717d4aa255d727301fed73103f294e12e16c7200daa4ac4126`。

可复查的命令在源码仓库内运行：

```sh
GOMAXPROCS=2 GOMEMLIMIT=3GiB ./node_modules/.bin/tsgo --project apps/web/tsconfig.app.json --noEmit --incremental false
cd apps/web
bun test --preload ./scripts/test-preload.mjs --timeout 15000 src/features/forge/forge-home.test.tsx
bun test --preload ./scripts/test-preload.mjs --timeout 15000 src/features/forge/home-directory-render.test.tsx
mise exec -- bun run build
mise exec -- bun run bundle:check
```

## 仍待完成

- 正式 Core 尚不能保存 `HeaderNavModules`，也未完整投影导航配置与公开服务地址。已有外置候选需要改为追加数据库升级，不能直接改写已安装组件的 SQL。该候选尚未集成、编译或完成数据库验收。
- 真实 Core 驱动的管理员保存、重新登录、刷新与重启后的读取，普通用户权限拒绝，以及桌面、390/320 像素浏览器验收仍待执行。
- 开发者页的发行者/请求示例、接入指南、脚本和分享卡仍有固定官方域名或品牌。OAuth 发行者必须按正式发现与信任合同处理，不能简单替换成浏览器域名。
- 本版本的完整 Web 回归、真实 Core/Go 业务流程、最终资金/兼容性/资源验收及部署尚未完成。

本批次没有操作生产服务器、生产数据或付费模型接口。

# 新架构进度

更新于 2026-11-15。此文件只维护当前状态、剩余阻塞和证据入口；执行过程、逐轮测试及历史失败保留在 Git 历史与对应验收收据。

## 当前状态

- **当前生产为旧 Go 0.2.118 与匹配旧 Web**：产线 owner 已在 Arch 回读 OldFormal PID 196448、服务 `active/enabled`，`api` 主站全路径切回旧架构；Terra 原有会话自然恢复，未新建登录、未清理 SiteData 或用户数据。dashboard、`api`、`status`、`data/self`、`log/self`、`stat` 与首页 7 项 JS/CSS 均返回 HTTP 200。这是当前承载状态，不代表新原生版本已上线；本轮 Git 收口不执行部署。

- **旧 Key 与余额核对**：产线 owner 回读 495 条普通旧 Key 已软删除，`public_remaining=0`；13 条 OAuth 与 1 条 internal runtime 凭据保留；实际回执为 `fresh69-forward-preparation-26752cdaae25/legacy-formal-recovery-20261115/old-key-retirement.actual.json`。核对快照中的两库 1249 位用户逐笔余额一致，总差额 `delta=0`；后续旧正式站真实消费按单独账单记录，不把该历史快照当作实时零差额。此结果只覆盖已核对的凭据范围与余额，不抵扣其他历史订单、订阅及持续权益验收。

- **Native69 受限影子已安装**：产线 owner 实际回读标准 `lmm-core 0.1.19-4`，PID 200728 执行新 `960196733396c24337eab69885ec2be14010aedfafd6a0fbdada3c1f3a3558f4` ELF；安装与 Core restart 均退出 0。Native ready 为 200、`setup=true`，operator 外仍 403，独立影子数据库与支付写入隔离保持。Native Web 已前向至 `0.1.149-3`，安装退出0，公网 index SHA256 为 `de2b857089631a1d9eb43cd326c117d1a4f9d087d84e718c583ddb0041206956`；Web 安装未重启任何服务；旧 Go PID 196448／0.2.118／正式 200、Extensions PID 197431、旧 Web current、config 与 Nginx 均未变。后端 GitHub `user:email` 已部署；升级前 Terra 新意图 start 返回 200 和 `success/data.authorization_url`，但旧 Native72ef 前端仍按 `read:user` 严格校验而未导航，这是实际前端反例。对应三行 Source 窄修及新 Native-only Web 一次构建/部署均已完成，新文档正常站点流程由 Terra 继续验收；真实 Root 登录、RW 与 Native 模型验收仍 NOT_RUN。安装只覆盖封闭 69，不是 CURRENT74 或正式新架构切流。旧 v3 首次 pacman SVG 冲突 FAIL、整事务未变及精确覆盖重试 0 的证据保留。最新安装证据：`dual-origin-20261115/native-installation/{core-960-installation-result.json,web-ghscope-installation-result.json}`。

- **浏览器会话恢复 v2（受限影子 Web 已安装，联合验证待跑）**：[`91a99d4`](https://github.com/TokenNotIncluded/lmm-api-source/commit/91a99d4) 只接入 owner 冻结的 13 个 Web 文件，保留原不确定操作 marker，通过新文档、同锁只读证明确认现有权限；仅显式点击续期可执行一次 refresh，并以匹配的终态只读证明完成恢复。逐文件摘要与 v2 manifest 一致，owner 自测 54 项及真实 Chrome 配合受控 HTTP 的 14 组场景通过，范围 lint、格式与七语言同步通过。该前端版本与真实 Core 的联合验证、共享 Web 全量构建与生产 Root 验证未运行；原 72ef 冻结 Web 包现已按实际回读安装于受限影子；独立 Core69 HTTP 结果不代替真实浏览器联合登录，旧 Go 主站未变。

- **只读 reconcile 与 RP 配置（源码已提交，固定 69 影子已安装）**：[`a0c814d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/a0c814d2b9a276848de6292110773da973bf7989) 保留原四文件只读会话核对入口，不写 Cookie 或新增权限；其静态与格式检查通过，Core/HTTP 验证未运行。[`28dbd00`](https://github.com/TokenNotIncluded/lmm-api-source/commit/28dbd00) 仅将 RP 的六文件 hunks 对齐当前源码，保留 Turnstile 守卫和其他未提交配置改动；允许安全父域 RP ID，两个验证器仍使用精确 Origin，依赖仅新增固定 PSL 与 psl-types。当前补丁适用性、范围、最小锁差异、overlay 摘要及依赖归档闭包核对通过；原 69 overlay 校验引用 Hooke 已应用补丁的可变候选基线，因 config.rs 漂移而停止，失败保留，未改哈希断言；另从不可变 `formal-release-preparation/core-bookworm-1794639540/published-compiler-inputs.tar` 仅读五个既有成员，匹配原 base 摘要并确认新增 RP 文件原先缺席，在内存应用原 diff 后六个 postimage 全部匹配冻结 overlay；未计算整 tar 摘要或重签；全文件格式检查仍报既有支付测试格式问题，RP 新增内容无需格式修改。当前 74 组件源码的七项 Rust 单测、Cargo 与实际 HTTP 验证未运行；下述固定 Core69 ELF 的七项 RP 配置用例不是这些 Rust 单测。源码提交与独立封闭 69 候选验收分别记录，不代表当前 74 整树已通过；实际仅安装下述固定 69 受限影子包。

- **旧5bc Native69 验收（历史38项通过）**：固定 ELF `5bc614a4a1c6e75843d648b0c30b065b9bba49828d3037a7cf850ed3dc86ee54`，runner V3 `6c7c3fa7ceb412604ff76404d23896ae6697a24ba4f443f617eb668fc1d1b51b`；manifest 的 69 组件与 catalog 分别匹配冻结预期。真实 HTTP 的 31 项 reconcile 全部通过，均无 Set-Cookie，全部认证表及 sequence 前后不变；7 项 RP 配置用例中，2 个合法父域启动、5 个非法配置拒绝。原 v1/v2 因 fixture CLI 与授权配置错误产生的真实 FAIL 保留，不记为 Core 业务失败。该 38 项不包含 RP 实签，新增软件 P256 六项结果另行记录；实体硬件、真实 MFA 第二因子正例、慢请求体入站压力与跨进程并发撤销仍未运行。本结果只绑定此固定候选及上述范围，不抵扣当前 74 组件整树、前端联合或生产验收；本次 runner 未构建、未触碰生产；该固定候选曾作为 0.1.19-3 受限影子 Core 安装，当前已前向至新960／0.1.19-4，正式旧 Go 未切换。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/run-8117d0a7ea1fe83d/RESULT.private.json`。

- **GitHub 最小 scope 修复（源码、新960门禁与受限影子安装已完成）**：[`947557c`](https://github.com/TokenNotIncluded/lmm-api-source/commit/947557ceee21994ea97fc89afb3a84f86f783fec) 仅修改 external_login provider 与既有 authorization URL 测试两个文件，将 GitHub `read:user` 恢复为原已授权的 `user:email`；其余 provider、state、PKCE、redirect 与 subject ID 实现不改，Rust 产品单测仍 NOT_RUN。新固定 ELF `960196733396c24337eab69885ec2be14010aedfafd6a0fbdada3c1f3a3558f4` 构建退出 0（636 秒、OOM=false），12 个 source 输入无漂移；新 31 HTTP＋7 RP 配置实际 PASS/exit0，原 phase3 内同 ELF 软件实签 6 项实际 PASS，独立 scope-only v2 再实际 PASS1/TERMINAL exit0（0.575 秒），资格合计为 38＋6＋1＝45。scope-only 用 accepted_legal=false／双 legal=false，实际校验唯一 user:email、DB PKCE S256、Client/Redirect、state digest 与 binding cookie double-derive；accounts/users/oauth_subjects/credentials/families/proofs 前后相同，无 Subject/Session 创建、无外部 GitHub 请求。原 phase3 因 fixture 缺必填 bool 而 400/aggregate exit1、scope-only v1 HMAC fixture FAIL 均保留；不重跑 38/6，不复用旧 5bc44。final cleanup PASS，所属 Core/PG/runner 已停、container 已移除，固定 ELF/输入/全部 PASS-FAIL/原8材料保留；builder 未触生产、未新增 Cargo。该产物已作为受限 0.1.19-4 影子 Core 安装，网站真实登录/RW/模型仍未验收，原8材料/硬件登录、真实 MFA 第二因子正例、慢请求体压力及跨进程并发撤销、CURRENT74 全树仍未运行。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/github-scope-only-v2-f057e676b3a83044/GITHUB-SCOPE-ONLY-RESULT.private.json`；`native69-github-scope-forward-1794698626-fbd978/{qualification-result.json,scope-only-v2-exit.json,final-cleanup-receipt.json}`。

- **旧5bc RP 软件实签（历史另6项通过）**：同一 `5bc614a4` ELF 与固定 PASS fixture，实际软件 P256 签名使用 `https://native.api.lmm.best` Origin、`api.lmm.best` 父域 RP；UP-only／UV=false 与 UV control 均返回 200 并获得 NativeSession。sibling、错误端口、错误 scheme、错误 RP hash 四项分别返回 401，Session family、proof、session credential 均不增长，存储 Key 与 revision 不变。正例 counter 从 137→138→139，存储的原 UV=false、BE、BS 保留。`rp-signed-v2-exit.json` 为 TERMINAL、实际退出 0、1.272 秒；RP runner 的 Cargo 次数为 0，supervisor 的累计构建 1 次来自之前唯一 ELF，原元数据不改。主 31 HTTP＋7 RP 配置未重复，不读取原 8 份材料，实体硬件与真实 MFA 正例未运行，未操作生产。实签初版在 review 阶段因 CLI 问题拦截，未执行，不记运行 FAIL。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/rp-signed-f843f332547a58e9/RP-SIGNED-RESULT.private.json`；该 gate 最终 cleanup 通过，所属 Core/PG/runner 已停，builder container 已移除，固定 ELF、冻结输入、全部 PASS/FAIL 证据保留，不影响生产或其他 runner。

- **旧正式站单次模型调用**：一条真实临时 Key 请求返回 200，输入 15、输出 4 token，实际扣费 19 quota／0.000038 USD；临时 Key 932 已软删除读回，私密文件与自有剪贴板已清理。仅证明这一次请求与扣费完成，不代表全部模型恢复；未追加调用。证据：`dual-origin-20261115/old-model-singlecall.private.json`。

- **CURRENT74 按 scope 收口，整树验收尚未运行**：注册／联邦 11 文件已按 Main 授权以 [`7a00fb2`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7a00fb2) 单独提交推送，精确匹配 Raman 正式采用的 final，包含两处新增测试，未重复 apply。状态为 STATIC_ONLY，编译与业务全 UNRUN；manifest/base/工作树/final/暂存摘要逐一匹配，材料 `/tmp/lmm-current-registration-closure/`，不混 http 单位、GitHub provider、协议或客户端 fixture。`http.rs` 的 13 个权威 credit 单位字段及对应测试已由 Main 静态采用，以 [`1684ee3`](https://github.com/TokenNotIncluded/lmm-api-source/commit/1684ee3) 仅两个语义 hunks 提交；scale/exact 均 500000，无虚构 CNY 汇率；当前 Rust 与唯一 CURRENT 实际 `/status` 验证仍 NOT_RUN，已装 Native69 不含该补丁，两处 FED formatter 保持 dirty。协议 owner 已冻结 encode.rs、decode/zconv.rs、relay_http.rs 的 exact 三文件供唯一 CURRENT 测试，`FROZEN_FOR_CURRENT_TESTING` 不等于业务 READY/PASS；base db13602、patch SHA256 `8d643e3abe16a9c70aa782455f15924b85884b3b0a3528bddf82210b57c2e919`，CURRENT-base patch 无额外 Go lifecycle hunk，完整文件从 base 继承部分另行验收。材料 `/home/lightjunction/.cache/lmm-protocol-routing-20261115/current-threefile-freeze/` 含 manifest/patch/目标 matrix、vision 与后续负载命令；旧 v2 FAIL/build UNKNOWN 保留，最新修复业务 NOT_RUN。可准备唯一 CURRENT artifact，新 Native Web 窗口优先，不同时构建、不另开 Cargo；Native69 资格不外推至当前整树。

- **助手 history v3（源码已推送，限定范围独验通过）**：[`ed71cb1`](https://github.com/TokenNotIncluded/lmm-api-source/commit/ed71cb16f8e4d74310412b864451c5fc6ef12bc1) 接入 Conf 精确 13 文件，正式 history 保留 human、conversation 外键、sequence、nullable 实际作者及原空标题／preview／消息空壳列表语义；显式 owner CLI 负责升级、恢复与冻结 projection，runtime 无 raw stage 权限。testdata 为公共 v2 DDL，内联数据仅合成 9001/9002/9003；两真实隔离 PG、三 Go 包与 CLI/格式/文档自测通过。Volta 首次独验发现 missing-owner/NULL 反例，原 FAIL 保留；[`f4be2bc`](https://github.com/TokenNotIncluded/lmm-api-source/commit/f4be2bcdc2395ac1fa61a4a26df193174f4a9176) 仅补修两个文件，以 JSON 类型与 NULL-safe 比较拒绝必填身份字段缺失/null/错误类型，未知 actor 仍 nullable。18 边界及原反例、真实 CLI/owner API、replay/rollback/unknown-lineage/ACL 独立复验通过，实际退出 0、PG cleanup 0，其余 11 文件未变；原 no-tests、fixture owner 和独验 FAIL 均保留。证据：`lmm-completion-20261113/ahv3-independent-2d938704/fix/RESULT.json`。独验使用 `Go1.27.4-X:nodwarf5`，不抵扣项目正式工具链资格；Shadow 正规升级/project、生产 DDL、重启或部署未执行，142 条 support 仅 raw 保留，正式 support 链未恢复。

- **OIDC token/userinfo 测试（Main 静态采用，PG 业务未运行）**：[`c65f578`](https://github.com/TokenNotIncluded/lmm-api-source/commit/c65f578) 仅提交 `oauth2_lmm_author.rs` 现有 test-only 增量（73 行新增、1 行替换），通过 issuer clone 保留引用，新增真实 router/token exchange 以及 GET/POST userinfo 的 no-store、仅 sub 断言，保留原 discovery/JWKS/bearer/disabled 断言。定向 rustfmt 与差异检查通过，实际 PostgreSQL/业务测试仍 NOT_RUN；Magpie budget、common fixture、注册闭包及其他 dirty 未夹入。

- **Magpie test-only scope 已分别推送，业务未运行**：Arch 明确认领的 with_magpie_fixture 四行 Arc/Send/static 签名与 recovery addendum 精确后像，以 [`6ad4279`](https://github.com/TokenNotIncluded/lmm-api-source/commit/6ad4279) 两文件收口；recovery SHA256 `1be6e2725d27458a50826a4c09f0cf9521e211e6f86fa25e190f84cbf4da5210` 匹配。Main 逐行采用的 budget 52 行 test-only diff 另以 [`db13602`](https://github.com/TokenNotIncluded/lmm-api-source/commit/db13602) 单文件提交，包含 owner 隔离 fixture 事务内临时停/复启 request_identity 并 assert O 的历史授权时间、developer 撤权拒绝/恢复、模型 dispatch 同 family/无 key_id/预算49断言；不冒真实 elapsed，不减少旧业务断言。全部实际 PG/模型业务 NOT_RUN；原实际 exit101、撤回排队重试与 recovery runtime UNKNOWN 保留。未纳入未知 docs/magpie-backend 或其他代码。

- **Native Web GitHub 严格 scope 窄修（新包已实际安装，真实登录待验）**：[`2031c38`](https://github.com/TokenNotIncluded/lmm-api-source/commit/2031c38d40b5f48e121c22cfa0a0197828d96423) 仅三个文件各一行，将 external-oauth.ts 的 GitHub preset 改为精确 user:email，两套 unit/hook fixture 使用独立字面量合同、不再跟随同 preset 生成。旧 scope 对新合同两套均实际 FAIL，修后 15 unit＋21 hook＝36 PASS，提交与 owner receipt 三文件摘要匹配；不放宽任意 scope、endpoint、Origin、Client/Redirect、state、PKCE S256 或 intent，无新 Core 改动。用户已授权以原72ef封印输入＋这三文件 overlay 在新目录进行一次 Native-only Web 构建/部署；现一次 build/bundle 均退出0，标准包 0.1.149-3 已交付（SHA256 `88479913315493b1619e8bd8871938e8b504fe172299b34152c311ab37c6e30c`），604个文件摘要及 compiled user:email 已核；这是 base+overlay 对应 Source patch，不冒充 whole HEAD。未全量 typecheck，原 base 三项 FAIL 保留；原 Native72ef/v2 产物、收据与真实 start200未导航反例保留。安装实际退出0，公网 index 摘要精确匹配 `de2b857089631a1d9eb43cd326c117d1a4f9d087d84e718c583ddb0041206956`；Core/Ext/旧Go PID 与旧正式 Web 链接保持，未重启服务。新文档真实 Root 登录/RW/Native 模型仍待 Terra 实际验收。证据：`native-web-v2-72ef687-20261115/oauth-github-scope-fix/{fix.patch,receipt.json}`。

- **Web 用量日志测试窄修（源码已推送）**：[`585e417`](https://github.com/TokenNotIncluded/lmm-api-source/commit/585e417) 只修一个测试文件的 sortOrder 字面量和新数组上的 reverse/find，21 项定向测试及修后唯一全量 typecheck 通过。没有更改 13 个 Web 产品文件、重建或回写旧 receipt；原 72ef 冻结构建的 typecheck 失败、build／bundle 通过记录保持原样。

- **原 Passkey 实际恢复（Native69 影子库）**：冻结源 8 条有效凭据、7 位原活跃用户已通过 NOLOGIN owner 守卫事务 COMMIT，独立事务完整读回匹配；原 credential ID、十进制 user handle、公钥 JSON、计数、UV 与 backup flags 保留，2 条 UV=false 和源 Preferred 策略不拔高。原空显示名按既有合同补标签；冲突不覆盖新登记。导入回执与审计各一条，未写 Session、密码、OAuth 或资金，未重启 Core。2315 兼容资格来自既有发布归档绑定；真实硬件登录未运行。私有证据仅存 `fresh69-forward-preparation-26752cdaae25/plans/passkey-recovery-20261115/RESULT.private.json`。

- **登录页留白改版**：已推送 [6ab724b7](https://github.com/TokenNotIncluded/lmm-api-source/commit/6ab724b7f78804cb67230e96f07779b1b156cb06)。登录表单采用居中 400px 布局、52px 控件与更大留白，底部小游戏按需展开；协议确认保留，重复说明移除。`user-auth-form.test.tsx` 47 项、`mobile-auth-presentation.test.ts` 1 项自测通过，均为 0 失败；改动范围 `oxlint` 与 `i18n:sync` 通过，locale 无差异。已在 1363×936 浅色桌面预览实际组件，测得表单 400px 且无横向溢出；已修正观察到的密码框与主按钮高度覆盖。后续浏览器连接超时，最终截图、移动端和深色复验尚未完成。全量 Web 类型检查两次均被环境 `SIGKILL` 终止（退出码 137），没有通过结论；合并 `a7e2730` 后生产构建与首屏检查通过（gzip 594,658 字节，上限 786,432），移动布局 1 项与范围 lint 复跑通过；独立验收与部署未执行。

- **前端对齐与缩放重排（源码已提交，未部署）**：[`70cdd3e`](https://github.com/TokenNotIncluded/lmm-api-source/commit/70cdd3ed253160dac56a05b8f1f5b28760580b77) 统一共享标题、正文和页脚边线，补齐短窗口滚动、表格操作换行及认证表单伸缩；[`a7e2730`](https://github.com/TokenNotIncluded/lmm-api-source/commit/a7e27301e9fe46be6b1f7105e3edd95af896f8b1) 让开发者与钱包工作区按容器宽度重排，并保留并行的新版页面、金额快捷选项和支付确认路径。共享布局、登录及表格 22 项用例修正旧样式位置断言后通过；合并后的开发者与钱包 34 项通过，最新钱包输入又定向复测 14 项通过（不重复计数）。固定 `a7e2730` 的生产构建、首屏包体检查及最终钱包/检查脚本 lint 通过，首屏 gzip 594,656 字节，上限 786,432。两次全量类型检查均被环境强制终止，没有通过结论；本地 Chromium 提权被自动审批拒绝，云端浏览器入口受阻，GitHub 尚未出现本轮检查运行记录。[设计规范](../apps/web/DESIGN.md) 与 [66 场景浏览器检查](../.github/workflows/layout-reflow-review.yml) 已准备；缩放截图和独立视觉验收仍未执行。原失败、复测与构建日志保留在交付包 `LMM-API-frontend-reflow-a7e2730.zip`。

- **共享界面与钱包留白整理（固定 `249ed44`，未部署）**：共享卡片、表单、表格、控制台与公开页面基础样式减少装饰边框和重阴影，扩大页面及区块间距；钱包突出余额、报价与主要操作，保留同期开发者工作区更新。[固定源码](https://github.com/TokenNotIncluded/lmm-api-source/commit/249ed44e9337ad35160f2a3703bf434f19d50c43) 的 Web 生产构建与首屏检查通过，压缩首屏资源 594,623 字节（上限 786,432）；最终登录与数据表布局定向自测 6 项通过，前一合并阶段登录、价格与开发者页 23 项通过。原始导航文案断言失败在变更前基线也能复现；钱包复测 8/8 通过。先前定向 lint 的两项既有 effect 问题、两次类型检查退出 137 及浏览器视觉检查受阻均如实保留；已做开发自测，独立验收未运行。静态构建、版本说明和原始日志已保存于交付包 `lmm-api-visual-refresh-249ed44.zip`，不代表生产或全站视觉验收通过。

- **前端视觉整理（源码已提交，视觉验收待完成）**：首页移除装饰性编号、坐标、光晕和重复宣传条，连接说明采用开放排版；登录注册保留原生表单与可选游戏，取消旋转环、玻璃内框和重复标题；定价页收敛标题与卡片层级，保留金额、单位、缓存价格和筛选。已合入共享界面留白改动，保留并行开发者页与钱包工作。实现提交的 22 项定向自测及改动范围 lint 通过，静态预览与生产构建通过，首屏压缩资源 595,203 字节（上限 786,432 字节）；全站类型检查两次被系统终止（退出 137），没有通过结果。Chromium 启动被环境拒绝，共享浏览器连接及桌面交互失败，截图与独立视觉验收未完成；没有部署。实现见 [`b55324f`](https://github.com/TokenNotIncluded/lmm-api-source/commit/b55324f3319b7ac1bded4faa7bb9ffff816b8a47)，界面规则见 [Web Design](../apps/web/DESIGN.md)。

- **Material 强调色与主题（本地，未部署）**：默认蓝色主操作、绿色正常、琥珀提醒、紫色身份、红色危险；用户列表补齐语义徽标、身份图标、金额层次，普通用户概览/问候区/侧栏随主题变化。主题预览展示完整色板，窄屏选项换行，保留字体/圆角/密度/布局定制。Web typecheck、build-web 与 bundle 检查通过，原生用户列表 12 项和主题预览 3 项自测通过；首次用户列表测试漏 preload 的失败保留，纠正命令后通过。范围格式检查通过；用户列表原有两处 set-state-in-effect lint 错误未改。Chromium 已检查概览、主题设置及海风预设刷新保存；证据 `/tmp/lmm-material-review/`。用户列表明暗色与 390/320px 浏览器检查因本地预览服务超时未完成；验证后该文件继续有并行折叠详情改动，既有通过记录不作为最终合并快照验收。独立验收、提交推送和部署未运行。

- **AI 助手入口与历史视觉（本地，未部署）**：页头改为文字胶囊入口，移动浮动入口采用语义配色和更大触控区域；新增 Ctrl/⌘ + Shift + Space 开关，忽略输入法组合与按键重复，保留草稿和旧快捷键。历史按日期分组，分离时间、标题、摘要与查看/归档操作区。助手/历史 86 项自测通过；页头修复后 9 项通过、1 项既有钱包标签期望不匹配保留。Web 类型检查、构建、bundle、格式检查通过；lint 有既有 import cycle 警告。1440px/390px 浅深色浏览器自测通过，320px 入口可打开，历史检查先受 debug 通知遮挡、随后预览服务超时，未完成。证据 `/tmp/lmm-assistant-review/`、`/tmp/lmm-assistant-tests.log`、`/tmp/lmm-assistant-header-tests.log`、`/tmp/lmm-assistant-build.log`。独立验收、提交推送及部署未执行。

- **支付网关补齐（本地候选，未部署）**：修复 Creem 展开式商品引用、EPay/Waffo 查询交易绑定及 Linux DO/CNY 主机隔离；EPay/Stripe/Creem/Pancake 新订阅购买开关与钱包分离，旧单读取和回调保留。新增 Creem 周期订阅、独立 Core/Go schema 与离线升级，接入 Stripe/Creem 购买恢复、取消续订及价格管理 UI、七语言。Pancake 外部退款回调绑定原交易与实际金额，自动退款申请继续关闭。退款持久查询任务保留原操作，不重复 POST；独立 PG/OS 进程复验发现 unsigned 成功观察会抑制 signed 失败，已修复观察隔离与旧 done 事件重投恢复。v2 固定产物通过真实进程重启及原失败库克隆恢复，原 key/hold/proof 不变且只释放一次；Core 为显式替身，不抵扣真实资金账本，原失败与复验均保留于 `/tmp/lmm-payment-independent-20261115/REFUND_LOOKUP_V2_ACCEPTANCE.md`。最新 Go 支付、RPC 客户端、装配、Web 接口、应用及安装工具 race 自测通过；43 项无 fixture 跳过不计通过。Creem Go/升级 CLI 隔离 PG 与两套 UI 定向自测通过；中央构建退出码未知且仅存旧可执行文件，本轮 Core UNKNOWN／未签发，真实账本用例未运行；后续须从不可变归档重新构建。商店四种外部支付方式仅有未注册 Core 草稿，目录、RPC、checkout、退款执行与收入回冲尚未接通，钱包结果不能抵扣。真实商户、全链路浏览器、生产启用与部署未验证，不能据此宣称全部网关已可用。

- **Native69 影子库 Root1 权限恢复**：按冻结源 Root100 的实际权限及四条显式 allow（无 deny），使用目标 `lmm_core_native_20261115` 的 NOLOGIN owner 事务补齐缺失的 `core.permissions.v1` header、九行权限和 canonical payload；直接编译 sealed69 编码函数确认 242 字节及哈希一致，并写入一次审计。该影子数据库读回通过，当时原生入口的现有用户 `/api/data`、`/api/data/flow`、`/api/data/users` 已由 403 恢复 200；独立验证 owner 随后使用原 Root1 会话确认用户目录显示 1249 位用户，Nginx 回读 `/core/v1/admin/users`（20:03:56Z）及 `/core/v1/admin/system-resources`（20:04:27Z）均为 200，管理统计/日志接口也为 200。Root1 代际与 MFA、导入事实保留；6/67 仍停用且无策略。未创建 Session、未读取 Cookie/token、未部署新 Core。证据：`fresh69-forward-preparation-26752cdaae25/plans/root1-permission-recovery-20261115/RESULT.json`。

- **旧架构运行兼容清理**：Web 的 OAuth state/绑定回退、嵌套状态包裹、旧 URL 别名、双架构钱包/订阅/促销页面和无规格商品购买分支已统一为原生合同；签到只接受原生精确金额与服务端日历，领取携带操作键，待办分页与未读计数由服务端提供。模型元数据使用原生路由管理，保留由 Extensions 支持的 io.net 部署。Core/Extensions 移除环境兼容启动模式，现行调用方改为显式 TOML；历史导入执行器、冻结结构、旧订单及资金权益合同保留。相关 Web/Go 定向开发自测及 Web 构建、bundle 检查通过；性能汇总仅使用原生计数器，不再平均旧响应，用户列表及待办跳转统一原生查询字段。全局 Web 类型检查仍有未改动的 `prompt-input.tsx:103` 未使用变量错误，部分既有 effect lint 错误保留；Core CLI 解析器验证通过，但最终定向 `cargo test --no-run` 退出 101：`billing/legacy_tiered/eval.rs:46` 的运行时代码引用了仅在 `cfg(test)` 下声明的 `Error::ClockRange`（E0599）；此前未产出有效 Core 测试二进制，失败证据保留；远端 `524033f` 的 ClockRange 运行时变体修复现已正常合并，本轮未重跑 Core 编译，完整运行、数据库及独立业务验收仍未执行。源码与父 gitlink 同步不抵扣这些验收，也不改变当前旧 Go 主站。

- **找回密码入口**：登录页将“忘记密码”设为独立链接，密码登录关闭时仍可访问；邮件找回页新增返回登录。登录组件 47 项自测、范围 lint 与 390px 本地浏览器往返跳转通过；浏览器使用合成接口，未验证真实邮件送达。当前全站类型检查仍受 `prompt-input.tsx` 未使用函数及并行 `route-aliases.test.ts` 类型错误阻断；未执行独立验收或部署。

- **计费通用运算复用**：普通 Token、按次、工具、grounding、上游成本、阶梯、图片及音频计费复用现有 `num-rational` 的精确舍入；图片按张计费以有理数库替代手写 GCD 和交叉约分；旧阶梯日历使用现有 `chrono`，保留固定 UTC+8、星期编号及原时间范围。无新增依赖，最低扣费、折扣顺序和溢出边界保留。新增半值、零费用、整数边界与闰日/上海午夜用例；共享 Core 测试二进制定向自测 105 项通过、0 失败，旧 Go VM 对照用例因缺少私有 `LMM_LEGACY_TIERED_ORACLE` fixture 按原设置跳过。格式、差异和文档检查通过；数据库并发与支付业务独立验收未执行。

- **登录注册专项开发自测**：统一注册与改密的 Unicode 密码长度边界；Passkey-only 保留用户名并阻止密码提交；登录、注册、找回密码在 Turnstile 过期及提交后清除旧令牌，邮件验证码发送失败也重建控件；原生联合认证不再把已替代的旧微信/OIDC 计为可用入口。定向组件回归通过；本地真实浏览器使用合成 API/Turnstile 验证三表单过期拦截、失败重试及窄屏 Passkey 入口（`/tmp/lmm-auth-browser-check.log`、`/tmp/lmm-auth-passkey-mobile.png`）。本轮构建及包体检查通过（`/tmp/lmm-auth-final-build.log`）；随后并行路由删除导致开发服务器热更新报缺失 `api-keys`/`settings`，不抵扣该后续工作区状态。真实第三方授权、邮件送达、实体密钥及独立验收未执行；全站类型检查被既有 `prompt-input.tsx` 未使用函数阻断，改密弹窗既有 effect lint 错误保留。

- **原生候选与当前源码分别记录**。已有独立封印的 69 组件原生产物及资源验收；当前源码包含尚未部署的 70 组件托管应用，生产 lib/bin 编译检查通过。此前 70 候选的严格 Clippy、27 项 PostgreSQL 与真实 600 秒恢复结果只覆盖其原固定输入，不自动覆盖新密钥隔离补丁。当前 Web 类型检查、生产构建及包体检查通过，定向认证、绑定、日期和 API Key 回归通过；全量 lint 仍失败，新组合全套未重跑。旧失败和 UNKNOWN 收据保留。Magpie 独立客户端与预算已纳入合同，原生插件及端到端接线仍未完成。
- **原生候选后续切换仍待完成**。当前保持旧 Go 与匹配旧 Web 对外服务；新原生版本须先固定可运行的来源与产物，再完成最终迁移核账、安装及切流。源码提交、候选构建和影子库恢复不等于已安装或已上线，未完成的功能与验收如实保留。
- **独立应用授权正在补接**。Core/CLI 共享的安装绑定、实时状态、有限模型预算和六百秒轮换恢复合同已独立审阅并推送；工作区第 70 组件及精确 69→70 升级正在实现、尚未提交；定向测试目标编译，以及授权、预算和 fresh70 数据库用例已通过；69→70 升级仍有两项失败，完整轮换恢复合同也未验完。此前 69 组件候选和测试只覆盖各自冻结范围。
- **历史 66 组件部署演练已跑通主要链路**。新装、`check-db`、九库备份与恢复、Core/扩展宿主启动及独立重启已有历史自测。当前 69 组件离线准入已独立终审，本地审核文件和双摘要已原样封印；身份迁移套件已通过（显式 PostgreSQL 夹具用例未运行），源码及审核文件漂移仍被拒绝；资金导入在此前 Native69 封印下完整 45 项自测通过，实际退出码为 0、输入无漂移；派发重试补修改变了 Core 源码摘要，新候选迁移准入需重新独立审核，旧封印不自动沿用；同库 Native69 身份导入、重放及负例已实跑；固定旧源码的新隔离 source 演练已完成；补充演练使用可验证 VCS 构建，完整 producer 前后 ACL、函数、序列及经济数据对照已捕获，进程真实退出码为 0，独立来源保留性验收已通过；完整金融审核仍未签署，历史 JSON 单位策略兼容补修已独立审核、自测并推送，新来源 archive/export 的离线身份 provenance、官方财务 plan/manifest 和完整单位事实工具已生成并实际退出 0，输入输出摘要及无批准标记已独立复核；仅历史 Native69 技术范围，新来源目标身份导入与金融批准仍未完成；空队列演练不证明非空 flush 或完整在途权益。尚未财务导入，不替代缺失的历史排空证据。正式迁移与最终停写演练仍未完成。
- **历史演练的身份与资金数据已导入并核对**。该快照目标包含 1247 个身份、4990 个账本账户、883 个 API Key、21199 条路由组、193423 条历史及 500 条 OAuth；保留 5 个托管项目、45 笔未决订阅责任及原钱包负债。真实 raw/sk Key 已完成本机 HTTP 鉴权，未外呼供应商。
- **八个历史扩展目标库已恢复**。store、support、payments、operations、community、acquisition 曾逐表比对；assistant、toolmarket 曾导入回读。商店与客服的用户引用错配已修复并推送，旧演练目标不能据此自动修复，需在全新目标重导验收。
- **完整模型/RPC 恢复配置已启动并 ready**。883 条 Key 映射保持不变；审核式重绑定解决了别名与原数据库名/OID/nonce 的绑定问题。
- **Native 上线阻塞仍在**：最终停写捕获、全阶段迁移与对账、部署切流、全站业务验收、负载/容量验收及独立验收未完成。旧快照 `FINAL_20261111T120751Z` 已失去切换资格。
- **旧商店与工具市场内容允许重建，不再阻塞上线**。密钥与 50 条库存、13 条工具凭据的只读验证证据保留，不再补旧内容恢复验收。已有订单及资金责任仍需核对；15 份历史非活跃订阅仍待最终候选复核。
- **历史支付义务仍未结清**：95 笔字段完整的充值依据已在本地读回；81 笔缺历史字段、13 笔 ePay 缺冻结付款快照及 3 笔孤儿记录继续保留未决权益与补证路径。60 条历史订阅订单保持 UNKNOWN，不视为已自动结算。充值合同见[旧充值义务](legacy-payment-obligations.md)。

- **Pancake 充值回调补修（本地）**：原生 `top_` 订单从持久化订单定位渠道，并用下单时冻结的店铺/商品解析回调；旧 webhook 地址不再按当前店铺误选原生订单渠道。停用渠道仍可处理旧单，跨渠道/币种/环境、畸形原生引用及重复 JSON 字段拒绝；原始回调字节继续交 Core 验签，HTTP ACK 不代表到账。新增定向路由与重复通知自测通过；使用内存存储和受控 Core，不代表真实商户、PostgreSQL 持久性或独立验收。原 Core 支付策略及旧渠道验证材料仍须保留，自动退款与订阅未在本次补修范围内。

## 首发范围

迁移工具本地补修：Passkey 缺少目标用户、Telegram 标识非法或重复时，历史 cutover planner 现在拒绝整批计划，不再跳过后继续导入。扩展 `report.json` 新增逐模块 `coverage`，分别列出非空归档、缺后端、待 Core 接管与丢弃计数；`reconciled` 仍只表示目标行数一致，`runtime_coverage_verified=false`，不能作为完整迁移结论。两套定向 Python 自测共 24 项通过；真实 PostgreSQL 导入及独立验收未执行。旧 MFA 恢复码、特殊 Key 重签、其他 Passkey 公钥格式及历史订单兼容缺口仍未解决；本次工具源码变化也不继承旧迁移封印。

本次迁移允许重新创建旧商店商品、库存及工具市场内容，不以这些旧内容及其凭据的完整恢复作为上线门槛；这不取消商店和工具市场功能要求，也不豁免已有订单、付款、退款、账本及持续权益的核对与恢复责任。

首发必须支持现有账号与密钥、文字和图片模型 API、充值计费（含人民币）、商店购买/库存/提货/退款、订阅及既有数据权益。未使用功能和非关键问题允许延期；Gemini 音频已延期，zcall 更新下版本处理。以下项目保留完整产品范围，未使用功能不自动成为首发阻塞。

## 验收状态

| 编号 | 当前状态 | 当前结论与剩余工作 |
| --- | --- | --- |
| REL-01 | 历史范围已验收 | 静态边界合同已验收；当前运行权限见 CHECK-07。 |
| REL-02 | 历史范围已验收 | 旧候选镜像资源与启动已验收；当前原生制品见 REL-03。 |
| REL-03 | 部分完成 | Core、Extensions、CLI 原生候选已构建并通过 ABI 检查；Core 69 组件原生初始化与受限 runtime 复验通过。剩最终 Web 组合、资源及迁移资格、签名/发布、生产安装与运行核验，CLI 分发见 CLI-01。 |
| AUTH-01 | 部分验收 | 正常 OAuth 回调、claims 拒绝、同会话重启与 refresh 轮换已有真实 TLS 浏览器证据；WeChat 绑定解绑、绑定锁竞争及用量读取权限已有独立证据。旧回调不覆盖所选账号，以及在途取消原/新家族撤销均已通过独立浏览器与 SQL 核验。独立负例发现仅删除取消标记可复活旧流程，以及无 Web Locks 时迟到响应可清除所选账号 Cookie；已补修并通过定向自测与新冻结限定安全复验；冷双标签页正常及取消仍通过，首次夹具失败保留。窄屏提示遮挡已补修并通过新构建七语言独立视觉复验。Cookie 恢复提示边界、过期旧凭证且刷新 Cookie 已丢失、旧 provider 配置迁移与正式配置仍待验。 |
| MODEL-01 | 部分实现 | 文字、视觉输入、Anthropic 缓存及 Responses 结构化输出转换已有独立验证；hosted tools/fileData 完整入口与转换覆盖待核实。 |
| MODEL-02 | 部分实现 | 原生 compact 与 JSON/SSE 加密重放已通过独立验收；exact 不支持路径在扣款前拒绝，JSON/SSE 检查点绑定实际发送字节。HTTP store/previous_response_id、授权与普通控制参数、tiered 计价（含非零 12 小时缓存）已通过固定制品独立 PG 验收；WebSocket 生产接线与旧行为等价仍待核实。本地 GET/DELETE 资源接口并非旧版要求。 |
| MODEL-03 | 部分实现 | Chat 音频多轮历史、计费与恢复已通过独立 PG 验证；独立 TTS/STT/translation 及双向 Realtime 未齐。Gemini 音频按决定延期。 |
| MODEL-04 | 待核实 | 视频、Kling/Jimeng、MJ、Suno 完整异步闭环尚无充分证据。 |
| MODEL-05 | 部分实现 | 渠道目录、余额探测、原生 tag 与迁移规划已有实现和本地证据；8 个在用模型已书面延期。演练中的文字、Responses、图片请求已完成，相关 reconcile hold 已按授权零结算释放；生产发布与完整业务验收未完成。 |
| ASSIST-01 | 实现待验收 | Core/Go 实时流、Web 历史恢复与取消主要链路已实现；助手/MCP/工具/HumanGrant 完整组合待验。 |
| SUB-01 | 部分实现 | EPay 一次购买路径已有；其他 provider 首购、周期、取消及恢复待逐项核验。 |
| SUB-02 | 部分实现 | 订阅管理记录与立即失效已有；voucher、绑定删除、批量重置完整行为待核实。 |
| PAY-01 | 部分实现 | 余额/免费商店购买与人民币钱包充值已有本地证据；外部商店收款及各外部支付方式的购买、交付、结算待验。 |
| PAY-02 | 部分实现 | 钱包退款与商店余额全额退款已有；外部商店退款缺冻结交易、通知/查询、收入回冲及恢复全链。 |
| PAY-03 | 部分实现 | catalog/pair/save 与 Core 绑定配置已有；更换 merchant/store/product 仍需 Core 策略同步，真实 Core Session、PG、外部 provider、浏览器及 OS 重启组合待验。 |
| STORE-01 | 部分实现 | 匿名提货、非零价购买→邮件→提货及 PG/产品重启已独立验证；丢 SMTP 确认保留 UNKNOWN 不重发，旧订单保持原登录边界。游客购买待补。 |
| STORE-02 | 实现待验收 | 新订单部分退款、历史余额退款授权导入及 Core/Go 协议已有；导入器已修复用户引用与账本账户 ID 混淆。当前程序集迁移准入、历史退款唯一返还及恢复的真实组合仍待验。 |
| STORE-03 | 部分验收 | 当前 Core/Go 的购买、提货、全额退款及重放已通过真实进程验收；足额自购、不足额拒绝、同钱包退款不增币、三种可见性和改私有后既有订单权益均已独立验证。完整浏览器与最终发布制品组合仍待验。 |
| STORE-04 | 待验收 | 账户隔离、购物车并发与清理有组件证据；最终组合的购物车、收藏、游客合并浏览器验收未完成。 |
| REVIEW-01 | 部分验收 | Go 审核与恢复链路已有真实 PG 证据；Core 分类/签名批准、最终跨语言组合和浏览器待验。 |
| APP-01 | 实现待验收 | GitHub 配置、加密凭据、原子拉取及快照已有实现；实际 Core 与浏览器组合待验。 |
| APP-02 | 实现待验收 | 实时用量来源、SVG 与 Core 历史查询已有实现；固定进程组合、浏览器及公网来源待验。 |
| APP-03 | 部分实现 | Go 目录、报价、购买/取消、投诉及管理接口已接线；Core 直接供应商调用、报价与资金状态待真实 PG 验收，正式供应商、进程组合与浏览器未验。 |
| APP-04 | 待验收 | io.net 实现及数据库权限缺陷复验通过；真实供应商、Core 授权与浏览器完整链未验收。 |
| APP-05 | 部分实现 | Core 管理、冻结报价、占用与入账消费及 66→67→68 分段升级已有 PG 验证；管理页、批量生成丢响应恢复、删除保留旧占用已通过独立浏览器/SQL 验收。provider 执行与历史优惠连续性待补。 |
| APP-06 | 部分实现 | 原生 abuse 策略、标记与审计已有；公开安全报告及 Moderation 旧行为映射待补。 |
| APP-07 | 待验收 | RSS 抓取、缓存、配置有组件证据；目标 edge 路由与浏览器未验收。 |
| APP-08 | 实现待验收 | 加密偏好、签名投递、持久重试和前端设置已有；旧用户偏好续接、旧待投队列及真实 PG/TLS/固定组合待验。 |
| APP-09 | 待验收 | 展示汇率的权限、保存与账本不变有组件证据；公网汇率源及完整页面链未验收。 |
| CLI-01 | 部分实现 | 只读登录、目录、余额、预设及 Pi 适配已有实现；客户端 managed lifecycle/CC Switch 已实现，Linux 官方安装与回执恢复已实跑。Core 安装绑定授权及轮换恢复已在工作区补接并通过定向 PG 用例；第 70 组件升级与完整恢复合同验收尚未完成；真实授权同步、会话恢复与多平台分发待验，旧 69 组件证据不自动覆盖新组件。 |
| CHECK-01 | 部分验收 | 旧身份、密钥、OAuth、Passkey、用户组及 raw/sk HTTP 鉴权已迁移核对；历史账号交互登录、实际模型配置及最终生产迁移待完成。 |
| CHECK-02 | 历史演练通过 | 旧钱包、欠额、推广余额、托管、订阅、订单及信任基线曾导入读回；当前 69 组件迁移准入、最终停写快照、生产对账与持续权益运行核验仍未完成。 |
| CHECK-03 | 部分验收 | API 密钥只读 HTTP 查询已核对；管理员 Session、统计/财务页面、时区展示和 CSV 导出逐项对照仍未完成。 |
| CHECK-04 | 部分验收 | 固定组合登录、人民币模拟充值、商店购买提货退款和恢复通过；历史未决付款、其他故障场景及生产配置运行待验。 |
| CHECK-05 | 部分验收 | release 文字与图片、计费及 UNKNOWN 路径通过；助手/MCP/工具/HumanGrant 完整组合与协议全覆盖未完成。 |
| CHECK-06 | 部分验收 | 固定组合浏览器用例通过；全角色/语言/窄屏、CLI 及 Extore 两端完整覆盖未完成。 |
| CHECK-07 | 部分验收 | 初始化、ACL、原生包校验及旧服务恢复已有证据；最终生产迁移、安装切换与恢复演练待完成。 |
| CHECK-08 | 待验收 | 同口径新旧版性能、长稳与故障恢复完整验收未完成。 |
| CHECK-09 | 部分验收 | 固定组合 HTTP、浏览器及 release 图片通过；QD-001 六项资金退出条件已独立复核关闭；实际历史账号、最终生产快照及目标主机验收仍未闭合。 |

## 证据入口

| 范围 | 证据位置 |
| --- | --- |
| Core7 制品与准入 | `lmm-core7-release-vh29l59q/admitted-core7.receipt.private.json` |
| 固定组合 | `lmm-combination-core5-web4-nt6y0hde/core7-go9-web5-combination.receipt.private.json` |
| release 图片 | `lmm-core7-release-vh29l59q/image-release-r2/result.json` |
| 当前 Web 构建检查 | `lmm-completion-20261113/web-build-only-candidate-1794657376/qa-receipt.json`；冻结 2,824 个输入、五项构建检查通过且无漂移；明确未运行完整测试，不抵扣 lint、业务或生产验收 |
| 当前财务导入完整自测 | `lmm-completion-20261113/financial-exit-verified-1794653222/receipt.json`；45 项、实际退出码 0、四个输入摘要不变，不替代真实旧快照导入与独立资金验收 |
| Native69 派发补修资源范围 | `lmm-completion-20261113/postfix-resource-preparefix-1794661473/REPORT.md`；实际原门槛通过，仅固定 69 组件候选；旧 boot 中断退出 UNKNOWN 保留，不是新 70 资格 |
| 补充来源与单位事实 | `lmm-completion-20261113/source-resume03-independent-1794659908/REPORT.md`；`~/.local/state/lmm-api/supplement03-native69-offline-tech-1794663446/attempt02/REPORT.md`；完整单位事实仅技术范围，独立核对见 `lmm-completion-20261113/source-fullfacts-independent-20261114/result.json`，无经济批准或目标写入 |
| 工作区 70 组件定向状态 | `lmm-completion-20261113/managed70-hooke-20261114/attempt06/compile-result.json` 与 `pg-attempt03/RESULT.json`；定向编译通过，授权/预算/fresh70 通过，升级两项失败及未运行合同保持未完成 |
| 身份与凭据 | `credentials-412-0e1e30sg`；Root 激活见 `trust-resumed-l2yw2und/root-owner-handoff/` |
| 资金及订阅 | `auth-continuity-rehearsal-8hqzzqo8` |
| 商店与客服 | `full412-go-producers-84jgu6ps` |
| 历史充值依据 | `cc52-payment-basis-l066cykb/normal-install95-local-02/result.private.json` |
| 历史查询导入 | `history412-readonly-j6_q5udc/applied-01/readback.private.json`；只读 API 见同目录 `http-01/result.private.json` |
| 信任基线 | `trust-resumed-l2yw2und/applied-01/result.private.json` |
| 市场审核 Go 恢复 | `pg-market-review-independent-ids9wytd/receipt.json` |
| HeroSMS HTTP 接线 | `herosms-go-tcp-independent.log` |
| 扩展注册与路由 | `go-plugin-targeted.jsonl`、`go-webcompat-final.jsonl`、`go-build.log` |
| Responses 旧行为范围 | `lmm-completion-20261113/model02-legacy-behavior-review.md`；对照固定 `go-v0.2.118`，源码核对，不是业务验收 |
| HTTP 连续性与缓存计费 | `lmm-completion-20261113/continuity-independent-b10-ed2d891190/HANDOFF.md`；固定 b10 的完整上下文、非零 12 小时缓存、重启/恢复已独立验证；`continuity-b10-wire-ack-1794596669/RESULTS.md` 仅覆盖 scalar/JSON 的三阶段真实 COMMIT 丢确认，不涵盖全部模式或真实付费供应商 |
| 原生 compact 开发自测 | `lmm-completion-20261113/compact-freeze-r3/report.json`；绑定最终格式候选；此前开发 PG 自测 9 项与共享输入回归 5 项通过 |
| compact 独立业务验收 | `lmm-completion-20261113/compact-independent-r3-e0f8c4ed/report.json`；最终制品独立业务 4 项、共享输入 5 项通过；保留原始两项失败与修复复验，不覆盖真实供应商或生产验收 |
| Core 模型与原生折扣码 | `lmm-completion-20261113/independent-core/report.json`；涵盖音频、Responses 转换、折扣码与 66→67 升级 |
| Web 折扣码报价与确认入队 | `lmm-completion-20261113/discount-independent-e924a11361/report.json`；真实 HTTP→Go→Core RPC→PG，不含 provider 执行 |
| Core 折扣码管理与 67→68 升级 | `lmm-completion-20261113/core-discount-management-68-report.md`、`core-discount-name-search-report.md`；四角色升级、数据/ACL 保留与名称搜索回归 |
| 折扣码管理页与批量恢复 | `lmm-completion-20261113/admin-browser/run-97fa78b790e7/result.json`；Core/PG/浏览器/SQL，含七语言窄屏与名称搜索；不含钱包 provider 执行 |
| 匿名提货 | `lmm-completion-20261113/independent-store/final/report.json`、`postgres-report.json` |
| 商店目录与结账回归 | `lmm-completion-20261113/go-store-independent-1096761-e5253ab52e/HANDOFF.md`；绑定 `1096761`，原失败组及完整 store PG 独立复验通过；合成资金边界，不抵扣真实 Core 钱包组合验收 |
| 旧密钥与库存密文回读 | `lmm-completion-20261113/server-crypto-recovery/recovery-receipt.json`；两服务器源端 50/13 解密及现有目标库存 50 逐条内容一致；无生产写入，不代表最终停写迁移或库存业务验收 |
| 提货邮件开发自测 | `lmm-completion-20261113/store-mail-delivery/delivery-report.json`；TLS/PG/worker/race 与 SMTP 认证，不抵扣真实付费组合独立验收 |
| 付费邮件独立验收 | `lmm-completion-20261113/independent-paid-mail/INDEPENDENT-PAID-MAIL-REPORT.md`；精确 Core 基线 + mail-only Go，含实际 PG/产品重启、TLS 与丢确认后实等；不含外部邮箱/真实供应商 |
| 支付签名过期与重启 | `lmm-completion-20261113/payment-expiry-79d9d4af2e73/report.json`；真实等待 302 秒，Go 进程重启、Core 服务对象重建 |

上述路径位于操作者私有缓存/状态目录，包含受限业务材料。仓库只记录证据位置，不提交秘密与用户数据。各收据仅证明其绑定版本与范围，不代表当前源码或整站已验收。

## 范围与维护

完整要求见[产品要求](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCT_REQUIREMENTS.md)、[生产验收合同](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md)与[支付矩阵](https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md)。历史审计见[归档索引](archive.md)。

只更新受影响项目的当前结果、具体剩余工作和必要证据入口，不追加逐轮执行日志。源码提交、本地验收、发布与生产部署分别记录。Extore 自动部署 main 限制继续有效。

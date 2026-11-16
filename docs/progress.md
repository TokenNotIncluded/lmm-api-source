# 新架构进度

更新于 2026-11-17。此文件只维护当前状态、剩余阻塞和证据入口；执行过程、逐轮测试及历史失败保留在 Git 历史与对应验收收据。

## 当前状态

- **钱包与账号密码登录补修（钱包 Web 已部署，真实收款尚未启用）**：钱包修复 [5d3a7b1](https://github.com/TokenNotIncluded/lmm-api-source/commit/5d3a7b13db115d23b56fc2094be987b5b9da49eb) 直接使用原生订阅读取，移除旧订阅和旧 topup-info 探测；订阅、奖励等次要区块按需挂载，已保存操作自动展开恢复。渠道或订阅读取失败留在所属区块，提供显式重试，不冒充空数据。余额突出显示有明确 v2 元数据支持的参考金额，同时保留精确 Credits；付款金额和到账额度继续使用冻结的 Core 合同。手机布局 [30ca41f](https://github.com/TokenNotIncluded/lmm-api-source/commit/30ca41fcf128d709bdc96255056683f7b8eeefa7) 将辅助统计收紧为一行。37 项钱包定向自测、类型检查、生产构建和包体检查通过；本地四组浏览器流程及两组最终手机布局通过，资金场景验证不确定结果后刷新只查询原订单、不重放付款。测试均使用合成账户和订单，不是实际收款验收。
  [部署 43930294571](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43930294571) 将固定 30ca 静态 Web 发布至 arch-shadow；首页、登录、钱包、状态及 7 个资源返回 200，未登录管理员接口 401，旧站 200，公网和回环 index 摘要一致。随后并行定价任务的 [部署 43933971760](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43933971760) 推进到 3c5617e / 0.0.0-preview.9；本轮 23 个钱包相关文件相对 30ca 无变化，当前 index SHA256 为 `02cb521964e8b93179260bbc9785f3a4711938363a8e36c077a6db27690d668c`。已发布 30ca 制品的三组受控浏览器场景通过；手机恢复的网络空闲等待及缺失 HumanGrant fixture 失败保留，制品变更时哈希守卫拒绝继续，不放宽版本绑定。当前固定 3c 制品的手机账号密码登录与原订单恢复回读通过；111 次实际联网均为本站静态 GET，全部账号/支付 API 使用合成响应，刷新未增加报价和确认之外的两次合成写入。最终回执为 `browser-wallet-deployed-3c5617-mobile-resume-run.json`。Core PID、Extensions PID 与支付配置摘要保持原值。证据：父仓库 `.git/lmm-wallet-20261117/`；独立业务验收未执行。

- **账号密码链路（同现网 Core 制品的隔离 HTTP 验证通过）**：Web auth 53 项定向自测通过；现网合成不存在账号的错误密码返回 401、无刷新 Cookie。使用匹配现网 1eba 的 ELF 和该制品初始化的隔离 74 组件数据库，正确密码登录、读取本人、刷新令牌/Cookie、登出、旧令牌及旧 Cookie 拒绝均通过，使用合成账号并已清理自有进程与数据库。这不覆盖真实用户会话、历史 bcrypt 导入升级或线上 HTTPS 全流程。两个当前源码 Rust 登录测试在编译 780 秒后超时，未进入测试，仍为 NOT_RUN。实际运行 ELF 的 schema 为 74；配置目录名 native69 不是当前 schema 证据。回执：开发机 `~/.local/state/lmm-api/password-core1eba-http-20261117/handoff.json`。

- **真实收款阻塞与权限候选（源码已推送，未部署）**：一次只读 provider 查询确认 5001/5003 共用 store，唯一生产 webhook 仍为 `https://api.lmm.best/api/waffo-pancake/webhook/prod`，进入正式旧 Go；SDK checkout 无每单通知地址，不能用浏览器 SuccessURL 替代回调。运行 Core/Extensions 的资金写权限、渠道及回调仍关闭。当前运行 Core 的 native/legacy 证明和入账共用权限，Go 还可恢复持久化旧回调。候选 [b8c5998](https://github.com/TokenNotIncluded/lmm-api-source/commit/b8c5998bed3893fbe1e39e690d45677a7c364f26) 新增默认 false 的 `legacy_proof_ingress` 与 `legacy_credit_evidence`，分离实际 RPC gate 和 capabilities，保留共同只读查询；配置样例和升级说明同步。格式、文档和差异检查通过，300 秒定向编译超时，未执行测试；整库存在并行输入变化，本轮七文件冻结摘要一致。Go 装配不强制 legacy 写能力，无需修改 Go。现网 1eba 不认识新字段，不能仅写配置先行启用；该候选没有编译/运行通过或上线资格。
  SDK CNY 数量报价序列化与原始 webhook 路由两个 Go 定向测试通过，使用合成 transport/Core fixtures，不代替实际账本闭环。已准备 native 精确回调和独立 store/新渠道的审阅候选，未修改 provider 注册或生产资金。仍需商户隔离决策、匹配制品的报价→checkout→签名回调→唯一入账/重启恢复验证，以及资金配置预览确认。证据入口：`payment-candidate/`、`provider-read/result.json`、`permission-split/final-receipt.json`，均位于本轮父仓库私有证据目录。

- **首页与浏览器会话恢复补修（此前部署记录，现行 Web 版本见上）**：固定 [9b34c07](https://github.com/TokenNotIncluded/lmm-api-source/commit/9b34c07bf1e0c45f06be18d9cbf31aedb11e05ca) 将公共页头统一为单一可读字标，移除首页重复 Logo，AI 导航改为与正文对齐的轻量链接，保留键盘、滚轮和触摸入口；手机目录边线错位已修正。恢复提示改为紧凑中性样式；修复崩溃后 pending 状态无法进入可恢复不确定态，以及应用启动清理提前删除会话恢复标记的两处问题，继续要求新文档、只读服务端证明与用户显式重新登录，不重放中断请求。上线回读另发现未迁移的脚本列表返回 503，已将该可选查询的失败与显式重试留在脚本区，停止全局 toast 与自动重试，保留服务端拒绝。67 项定向测试、Chromium 受控 HTTP 的 16 组协议场景、5 组首页/登录布局、2 组完整应用恢复场景及 2 组脚本局部失败/显式重试布局通过，类型、范围 lint、生产构建与包体检查通过；首次失败和修复证据保留。[部署运行 43914606492](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43914606492) 仅更新 `arch-shadow` 的静态 Web，实际制品 `0.0.0-preview.6` 与提交相符，公网与回环 index SHA256 均为 `8b50917caef7a3e50939c1bdeedb9b6ce679b6bdab3f0492ab3967f09a9344a7`；首页、登录页、状态接口及 7 个脚本/样式资源均为 200，未登录管理员接口仍为 401，旧站首页为 200，Core PID 与运行配置摘要未变。隔离匿名浏览器在当前 Core 上完成 pending 转 uncertain、新文档只读 reconcile 200、显式重新登录及标记清除，确认恢复前无 refresh 重放；该结果不覆盖已有账号的完整登录与续期组合。同包历史功能前端不据此取得后端部署或完整业务验收资格。开发与上线回读证据在父仓库 `.git/lmm-ui-20261117/`；独立业务验收未执行。

- **Google 登录按用户要求暂停**：用户明确先使用账号密码登录，待切换正式域名后再接入 Google。原私有 provider 候选未激活，OAuth 配置未变更。此前 native 回调的 redirect_uri_mismatch 诊断保留；正式域名确定后再核对最终 callback、client 配对与真实授权。历史证据：开发机 `~/.local/state/lmm-api/native-google-restoration-20261117/`，仓库不记录秘密或会话材料。

- **Native 入口 403 已恢复（2026-11-16，访问配置修复）**：公网诊断请求与 Nginx 的 access forbidden by rule 记录对应，当前操作者出口未被旧 IP 白名单覆盖；Core、前端文件及回环 HTTPS 原本正常。仅在 `/etc/nginx/conf.d/lmm-native-shadow.conf` 的 native 域名下新增一条当前操作者允许规则，保留共享 operator ACL 与 deny-all，QA 配置不变。Nginx 配置检查及热重载均退出 0，Core PID 未变化。开发机经公网复验首页、登录页、状态接口及 7 个脚本/样式资源均为 200；未登录管理员接口仍返回 JSON 401，旧站首页为 200。该入口继续采用操作者白名单策略，不代表公开开放或新版本上线。服务器备份与收据：`/root/.local/state/lmm-api/native-access-repair-20261116T153930Z/`；开发机结果：父仓库 `.git/native-403-20261116-verification.json`。

- **管理员用户详情统计合并（源码已推送，未部署）**：[a19e12fd](https://github.com/TokenNotIncluded/lmm-api-source/commit/a19e12fdbd4b7de96952d5136cc01456b43fbfc5) 保留个人资源用量与资金来源统计，另列调用者（actor）范围的模型活动、邀请与导入原始计数；模型活动包含团队资源调用及可用导入历史，与个人用量可能重叠，分开展示、不作总计。累计推广奖励、当前推广余额与历史原始值也保持各自来源和单位。数据库 SUM 超过 i64，以及用户 ID `9007199254740993` 实际结算后详情拒绝的回归均已由失败转为通过：聚合返回精确十进制字符串，详情专用读取支持合法大 ID；旧数字 API 边界及金额、退款、账本命令和收据守卫不变。隔离 PostgreSQL 定向通过管理员 36 项、community 6 项和原有 usage 集成 22 项，all-targets Clippy `-D warnings`、Rust 格式与差异检查通过；Web 28 个文件、175 项通过，格式、版权、lint、类型、生产构建及包体检查通过，gzip 589276 字节。本地浏览器覆盖 60 个布局状态、普通用户拒绝访问 1 项与展开明细 9 项，共 70 个状态；普通角色 debug URL 入口的初次失败与修复后的 clean-route 403 分别保留。视觉与交互检查使用本地只读 synthetic fixtures，尚未完成管理员浏览器到真实 Core、数据库的整链独立验收。

- **管理员用户详情接入原生用量（源码已推送，未部署）**：[733948f](https://github.com/TokenNotIncluded/lmm-api-source/commit/733948f8f165e5bd5da7bf06da8241fad53f3f9c) 按目标用户及其个人资源归属读取 Core 原生请求，汇总已结算、待结算请求数以及结算额、退款额和净消费，并区分个人钱包、团队代付、订阅与赞助。团队归属 Key 与历史导入记录不计入个人原生用量。详情在同一只读一致快照中读取，复用账本收据校验，累计金额以 u128 计算并返回十进制字符串；无用量与统计不可用保持不同状态。Web 使用摘要加折叠明细，七语言各新增 10 个词条并通过 helper 同步。最终 Core 定向 5 项、Web 定向 35 项、Core all-targets Clippy、相关 Rust 格式、Web 类型、范围 lint 与全量格式检查通过；生产构建及首屏包体检查通过，gzip 587314 字节（上限 786432）。本地 Chromium 的 1440/390/320px、英中俄、明暗主题共 8 组检查通过，形成 16 个摘要/展开截图，未见横向溢出；页面使用合成只读 fixture，数据库验证使用隔离 PostgreSQL 18.6，尚未运行真实管理员会话到数据库的整链路独立验收。首次失败与修复证据保留。

- **Responses 与当前历史迁移仍有未闭合范围**：既有 responses_continuity_independent_store_tristate_and_exact_refusal 在隔离 PostgreSQL 18.6 中精确运行通过；旧 CI 的失败没有完整断言差异，不能认定已修复，本轮未据此修改产品或测试逻辑。社区导入仍只接受冻结 Core 55/56 目标合同，当前 Core 75 的完整历史导入与升级资格尚未闭合，冻结指纹和目标白名单保持不变。原生订阅空态的格式错误已在 [0f72236](https://github.com/TokenNotIncluded/lmm-api-source/commit/0f72236e38c2db17e0adda5bdfb59e7ed886eb36) 修正。

- **原生钱包、用户详情与实例状态已补齐一批功能（已推送，未部署）**：[b45a89f](https://github.com/TokenNotIncluded/lmm-api-source/commit/b45a89f0842c7000dbe3ebf29e4dabc6d40f6ea6) 加入订阅原报价的只读收据恢复；[94ddf72](https://github.com/TokenNotIncluded/lmm-api-source/commit/94ddf72cbaf279ecf645769f13bc403bd8b62fcb) 补全钱包转移的准备、确认、领用、取消、列表与原请求恢复。未知结果保留原请求，不自动再次发送资金写入；恢复按账户范围绑定。[5e3cf1f](https://github.com/TokenNotIncluded/lmm-api-source/commit/5e3cf1ffe82108d111f667bc2e59b22e5526145b) 增加精确字符串用户 ID 的只读详情，保留管理授权与账户隔离，使用记录可以打开资料、访问状态与钱包信息。[c6b6d00](https://github.com/TokenNotIncluded/lmm-api-source/commit/c6b6d0061d39eaf09cbb298ee8a4b786238075a1) 使系统状态只展示当前存活实例，停止实例不留下停机历史项。

- **开发机真实浏览器完成局部界面验收**：Chromium 覆盖六个管理页面及钱包转移、用户详情弹窗的 40 个状态，含 1440/390 像素亮暗主题与 320 像素亮色布局。七语言移动端的 42 组合初跑中，40 项通过、2 项俄语按钮裁切失败；[d184ac9](https://github.com/TokenNotIncluded/lmm-api-source/commit/d184ac9dda2cc68b2d994f0716baf925fbff3785) 修复换行后俄语六项复测通过。相关 API、金额恢复、范围隔离、弹窗测试，以及类型检查、生产构建和包体门禁通过。这批截图使用对应所有者的管理员合成 persona，不能替代所有实际角色或完整新旧站点等价验收；后续用户统计与角色补测另按提交记录。

- **冻结历史升级已用真实历史程序验证**：[74731da](https://github.com/TokenNotIncluded/lmm-api-source/commit/74731dad3ba30f11d90c0aa8d7938e69cf4344d9) 将 Core58/67 执行器与固定源码、二进制和 manifest 绑定，补回旧版本精确 SQL，不修改冻结审核指纹。八个历史升级场景与一个私有回执边界用例通过真实 PostgreSQL；Core67 的迁移、重放和拒绝验证已通过。社区连接创建在实际 Core/Go 与独立数据库通过，CI 也提供独立随机角色与空库。范围格式、all-targets Clippy、45 项源码边界、15 项本地发布、18 项工作流和固定版本 actionlint 通过。该结果仍不代表历史社区导入后当前全栈或 Core 整条 aggregate 已通过。

- **财务开账与审核夹具回归完成**：[e9b98ed](https://github.com/TokenNotIncluded/lmm-api-source/commit/e9b98edd9faa10691b6c72011eed788991461d50) 修复四类规范初始 seed 被误当作业务写入的问题，完整 seed 指纹、字段/行数、余额、权益、权限和回执守卫保留。当前 Python 在固定审核 Core69 上完整发现 360 项：336 通过，24 项因未提供匹配 Core init-db Unix PG18 私有收据按原条件跳过；Docker PostgreSQL 18 已实际运行，耗时 575.169 秒。同次调用复用审核元数据后，本机单次 profile 从 2.485 秒降至 0.363 秒，合同完全相同；真实源码/别名修改和缺文件再次调用仍拒绝，没有跨调用缓存。资格仅覆盖“当前 Python + 固定审核 Core69”，不授予当前 Core75 财务导入资格。原失败和诊断中断记录保留。证据位于开发机 `out/lmm-completion/financial-final-reviewed69/`、`core-h.json`、`core-i.json` 及对应定向日志。

- **原生渠道与系统资源界面补全（源码已推送，未部署）**：[dac23b4](https://github.com/TokenNotIncluded/lmm-api-source/commit/dac23b43cc50961739b3ff2f8c1cc17a7bc77f87) 将渠道模型绑定、上游映射、优先级、权重、超时与供应商合同接入结构化表单，继续共用原 JSON 草稿、版本比较及单次保存；新模型不继承其他模型的合同，共享上下文只在最后一个绑定移除后清理。[d6ad78d](https://github.com/TokenNotIncluded/lmm-api-source/commit/d6ad78d1f00b75528ac391e0e73fcf09180cc608) 展示主机 CPU、负载、内存和 Core/Extensions 版本、进程、线程及采样信息，区分零值与未报告，并保留多实例资源归属限制；七语言同步完成。此前独立静态复核未发现可证实的新回归；本轮已补做本节记录范围的开发机浏览器检查，新旧版完整等价验收仍未完成。

- **本轮 Web 全量检查与构建通过**：固定 [a960937](https://github.com/TokenNotIncluded/lmm-api-source/commit/a960937cd01196b9a6a7025181077b8b85e3be5a) 的 [CI 43838128709](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43838128709) 已通过格式、版权、lint、类型、翻译回归、760 个源码测试文件（5303 pass、0 fail）与全部 6 个脚本文件；生产构建和首屏包体检查通过，gzip 587044 字节（上限 786432），已保存对应 Web 制品。首次 [CI 43835912453](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43835912453) 的 5302 pass、1 fail 保留：排序测试按全局下拉框顺序误选新增模型控件，a960937 改按实际列表控件标签定位，取消响应、币种、快照和草稿断言未删，产品代码不变。Core 全量仍未通过；该次运行取消前的失败记录保留，本轮已按本节原生功能、精确历史升级和财务合同分别复验，不能以 Web 通过或定向回归代替后端整条 aggregate 与整站验收。

- **CI 基线修复已推送，未部署**：Web 过期测试合同与合成数据已在 [8de640d](https://github.com/TokenNotIncluded/lmm-api-source/commit/8de640d4bc0f2d535fec36842b1973ba2da67ccb) 修正，保留金额未知、授权与历史计费断言。全量初跑中 753 个源码测试文件报告 5273 pass、0 fail；唯一失败为仓库位于 /tmp 时 operator 脚本测试提前命中正确的部署目录守卫，已在 [5e4242c](https://github.com/TokenNotIncluded/lmm-api-source/commit/5e4242c) 改用独立工作目录，4 项定向及全部 6 个脚本文件复测通过。复测源码树与初跑相同；没有宣称修复后重跑整条 aggregate。Web 类型、格式和 lint 检查通过，生产构建与首屏包体检查退出 0，首屏 gzip 586917 字节（上限 786432）。随后 b36872d 对应远程 CI 43763106737 的 Web 检查和生产构建也已通过；该轮 Core 任务被取消，不能视作 Core 通过。以上仍不是独立业务验收。

- **Core 验收工具与隔离认证修复**：[0ab8578](https://github.com/TokenNotIncluded/lmm-api-source/commit/0ab8578346f3278f6eee98dd349e91c1c8ac530b) 将过期 26 组件清单替换为编译合同与数据库逐项名称/SQL 指纹精确匹配，45 项 fixture 检查通过；[0764c94](https://github.com/TokenNotIncluded/lmm-api-source/commit/0764c941fceedf1fc236d66a38b610d63155c5f3) 精确发现封存付款用例，5 项回归通过并接入 CI。[3961245](https://github.com/TokenNotIncluded/lmm-api-source/commit/39612454502049e5e9c5820822bb4e64e4c28c67) 为隔离 installer/runtime 配置独立临时密码，真实 PostgreSQL 18 SCRAM 场景 1 项通过；[98f4ddb](https://github.com/TokenNotIncluded/lmm-api-source/commit/98f4ddbd7fff65825415b74e7e909c780272f4d5) 恢复历史升级安装器提示，真实 CLI 3 项通过。临时数据库已清理。原合成支付失败现已定位并修复，结果见下一项；历史迁移与 Core 全量验收仍未闭合。

- **真实合成支付闭环已通过，非生产支付验收**：[7d29a2d](https://github.com/TokenNotIncluded/lmm-api-source/commit/7d29a2df28ac2b0bc5a8d050737a13ca4173a2dd) 修复隔离测试数据库漏装订阅收件表及其最小权限，保留生产运行时存储隔离检查。封存当前源码与二进制后，实际 Core/Go 进程及 PostgreSQL runner 退出 0：两份已接受证明、两条收件记录只产生一份入账回执和一笔 journal，钱包增加 500000 单位、预留 0、非平衡 journal 0；重启 Go 后重复事件未再次入账，错误模式无证明/资金写入。子进程、临时数据库和私有目录均已回收。范围仅合成 bootstrap provider transport；正式公网回调入口、checkout/refund、操作系统重启和正式发布产物门禁未验收。[2eeafae](https://github.com/TokenNotIncluded/lmm-api-source/commit/2eeafae72e1ca7a1bd26cdb0203cdc475f6f1dfa) 增加只记录固定源码位置的失败诊断，7 项发现/脱敏检查通过，不保存私有原始日志。

- **托管恢复密钥隔离回归修复**：[824ab71](https://github.com/TokenNotIncluded/lmm-api-source/commit/824ab718aa7e3fd49c6f6eebe90cd299a3dac170) 使测试生成真实 TOML，并在转换时解开 serde_json 任意精度数字的内部包装；TOML 写入功能只作为 dev-dependency 开启。六项实际 Rust 测试全部通过，保留跨用途材料复用、旧密钥、HMAC 等效密钥、Telegram 派生、硬链接/路径别名及不生成未配置密钥的安全断言。生产加载器和隔离策略未放宽。当前开发证据在 /tmp/lmm-native-iteration/；历史升级测试仍试图用当前 SQL 重建冻结安装器，与当前生产拒绝策略不一致，未改写历史摘要或移除验收。

- **开发机连接与浏览器验证已恢复**：已通过 SSMCP 在原开发工作区继续开发，保留原有未提交修改与恢复材料。开发机 Chromium 已完成本轮管理员只读合成样例的界面与交互检查；此前 `UNAUTHORIZED`、Chrome `timeoutMs` 和沙箱启动失败只保留为历史失败。实际覆盖和边界见本节新增记录；没有把局部页面检查计为全角色、新旧站点完整等价或生产验收。

- **通用影子部署工作流**：环境变量配置任意数量节点，支持手动触发及 `v*` / `shadow-v*` tag。手动 Web 部署已在 ArchDmit、DmitUbuntu 完成（[43740218353](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43740218353)），实际浏览器会话恢复通过，未提交登录凭据。tag `shadow-v0.1.151` 的 Core、Extensions、Web 构建均通过，但[部署被 Core 兼容检查拦截](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43740351841)：新版返回 `CHECK_DB_SCHEMA_UNVERIFIED`，旧版使用同一账号配置检查通过；未切换或重启后端，候选文件已清理。尚未升级影子数据库，后端更新待完成。工作流校验归档来源、运行程序及 Web 字节，失败回滚，成功清理旧槽和上传包，不累计历史版本；配置见 [automation README](../deployment/automation/README.md)。当前仅影子环境，未切生产流量；开发自测完成，独立业务验收未运行。

- **设置页界面重排（源码已提交，未部署）**：[`35aebd9`](https://github.com/TokenNotIncluded/lmm-api-source/commit/35aebd98a4f5a878a5af21dacb75429413ad0506) 统一设置目录、标题/表单/保存区对齐，品牌与地址直接编辑，页面内容和协议按需展开，助手改用下划线标签；[`58ad5b8`](https://github.com/TokenNotIncluded/lmm-api-source/commit/58ad5b8fdd1ce7ef0b288f67f7251547e409a9c7) 同步只读值排版与设计规范。导航 5 项、权限/草稿/保存刷新 13 项开发自测通过；只读排版补充后相关 13 项再次通过，不重复计数。范围格式和差异检查通过，lint 无错误、1 条 refs 警告，i18n 同步无语言文件变化。固定 `58ad5b8` 的生产构建与包体检查通过，首屏 gzip 586,190 字节（上限 786,432）；构建记录随交付保存。两次全站类型检查被环境 SIGKILL 终止（137）；系统 Chromium 启动受限，官方 headless-shell 下载失败，云端浏览器无法访问本地预览，真实截图与视觉自检尚未完成。已做开发自测，独立验收未运行。

- **个人每日签到验证已接入源码**：Web/Go/Core 增加 120 秒随机挑战，默认每次新领取完成 Turnstile 和 Passkey 用户确认；Core 在同一事务中消费授权、记账并保存包含零奖励的每日回执。重试保留原操作、日期和金额，已到账优先恢复。新增第 75 组件及精确 74→75 离线升级，历史组件指纹保持不变。验证结果、启用配置与部署顺序见[签到防自动化](checkin-anti-automation.md)。本轮仅交付代码，不改变当前生产架构或替代生产验收。

- **当前生产为旧 Go 0.2.118 与匹配旧 Web**：产线 owner 已在 Arch 回读 OldFormal PID 196448、服务 `active/enabled`，`api` 主站全路径切回旧架构；Terra 原有会话自然恢复，未新建登录、未清理 SiteData 或用户数据。dashboard、`api`、`status`、`data/self`、`log/self`、`stat` 与首页 7 项 JS/CSS 均返回 HTTP 200。这是当前承载状态，不代表新原生版本已上线；本轮 Git 收口不执行部署。

- **旧 Key 与余额核对**：产线 owner 回读 495 条普通旧 Key 已软删除，`public_remaining=0`；13 条 OAuth 与 1 条 internal runtime 凭据保留；实际回执为 `fresh69-forward-preparation-26752cdaae25/legacy-formal-recovery-20261115/old-key-retirement.actual.json`。核对快照中的两库 1249 位用户逐笔余额一致，总差额 `delta=0`；后续旧正式站真实消费按单独账单记录，不把该历史快照当作实时零差额。此结果只覆盖已核对的凭据范围与余额，不抵扣其他历史订单、订阅及持续权益验收。

- **Native69 受限影子已安装**：产线 owner 实际回读标准 `lmm-core 0.1.19-5`，PID 202997 执行新 `66ead3852418bc25c4562c20902676e60c469113a900494c9c55c3b29388e9a6` ELF；安装与 Core restart 均退出 0。Native ready 为 200、`setup=true`，operator 外仍 403，独立影子数据库与支付写入隔离保持。早期 Native Web 曾前向至 `0.1.149-3`，安装退出0，公网 index SHA256 为 `de2b857089631a1d9eb43cd326c117d1a4f9d087d84e718c583ddb0041206956`；当前静态 Web 版本与回读见本节 2026-11-17 更新，Core69 资格边界保持原记录。该次 Web 安装未重启任何服务；旧 Go PID 196448／0.2.118／正式 200、Extensions PID 197431、旧 Web current、config 与 Nginx 均未变。后端 GitHub `user:email` 已部署；升级前 Terra 新意图 start 返回 200 和 `success/data.authorization_url`，但旧 Native72ef 前端仍按 `read:user` 严格校验而未导航，这是实际前端反例。对应三行 Source 窄修及新 Native-only Web 一次构建/部署均已完成，新文档正常站点流程由 Terra 继续验收；旧 Core4 的 API callback401 真实反例保留；Core5 已部署 issuer 修复，Terra 新文档新意图已真实完成 GitHub Root 登录，start/callback/self/history/stat 均200、读回8行且系统信息Online/Ready。Users API200但UI DTO不兼容待修；Resources独立route UNKNOWN，写操作与Native模型仍NOT_RUN。安装只覆盖封闭 69，不是 CURRENT74 或正式新架构切流。旧 v3 首次 pacman SVG 冲突 FAIL、整事务未变及精确覆盖重试 0 的证据保留。最新安装证据：`dual-origin-20261115/native-installation/{core-66ead-installation-result.json,web-ghscope-installation-result.json}`。

- **浏览器会话恢复 v2（历史版本记录）**：[`91a99d4`](https://github.com/TokenNotIncluded/lmm-api-source/commit/91a99d4) 只接入 owner 冻结的 13 个 Web 文件，保留原不确定操作 marker，通过新文档、同锁只读证明确认现有权限；仅显式点击续期可执行一次 refresh，并以匹配的终态只读证明完成恢复。逐文件摘要与 v2 manifest 一致，owner 自测 54 项及真实 Chrome 配合受控 HTTP 的 14 组场景通过，范围 lint、格式与七语言同步通过。该固定历史前端版本当时未运行真实 Core 联合、共享 Web 全量构建与生产 Root 验证；原 72ef 冻结 Web 包曾按实际回读安装于受限影子。当前 Web 构建、恢复补修与开发自测见本节 2026-11-17 更新；受控 HTTP 及独立 Core69 HTTP 结果均不代替真实账号与 Core 的联合登录，旧 Go 主站未变。

- **只读 reconcile 与 RP 配置（源码已提交，固定 69 影子已安装）**：[`a0c814d`](https://github.com/TokenNotIncluded/lmm-api-source/commit/a0c814d2b9a276848de6292110773da973bf7989) 保留原四文件只读会话核对入口，不写 Cookie 或新增权限；其静态与格式检查通过，Core/HTTP 验证未运行。[`28dbd00`](https://github.com/TokenNotIncluded/lmm-api-source/commit/28dbd00) 仅将 RP 的六文件 hunks 对齐当前源码，保留 Turnstile 守卫和其他未提交配置改动；允许安全父域 RP ID，两个验证器仍使用精确 Origin，依赖仅新增固定 PSL 与 psl-types。当前补丁适用性、范围、最小锁差异、overlay 摘要及依赖归档闭包核对通过；原 69 overlay 校验引用 Hooke 已应用补丁的可变候选基线，因 config.rs 漂移而停止，失败保留，未改哈希断言；另从不可变 `formal-release-preparation/core-bookworm-1794639540/published-compiler-inputs.tar` 仅读五个既有成员，匹配原 base 摘要并确认新增 RP 文件原先缺席，在内存应用原 diff 后六个 postimage 全部匹配冻结 overlay；未计算整 tar 摘要或重签；全文件格式检查仍报既有支付测试格式问题，RP 新增内容无需格式修改。当前 74 组件源码的七项 Rust 单测、Cargo 与实际 HTTP 验证未运行；下述固定 Core69 ELF 的七项 RP 配置用例不是这些 Rust 单测。源码提交与独立封闭 69 候选验收分别记录，不代表当前 74 整树已通过；实际仅安装下述固定 69 受限影子包。

- **旧5bc Native69 验收（历史38项通过）**：固定 ELF `5bc614a4a1c6e75843d648b0c30b065b9bba49828d3037a7cf850ed3dc86ee54`，runner V3 `6c7c3fa7ceb412604ff76404d23896ae6697a24ba4f443f617eb668fc1d1b51b`；manifest 的 69 组件与 catalog 分别匹配冻结预期。真实 HTTP 的 31 项 reconcile 全部通过，均无 Set-Cookie，全部认证表及 sequence 前后不变；7 项 RP 配置用例中，2 个合法父域启动、5 个非法配置拒绝。原 v1/v2 因 fixture CLI 与授权配置错误产生的真实 FAIL 保留，不记为 Core 业务失败。该 38 项不包含 RP 实签，新增软件 P256 六项结果另行记录；实体硬件、真实 MFA 第二因子正例、慢请求体入站压力与跨进程并发撤销仍未运行。本结果只绑定此固定候选及上述范围，不抵扣当前 74 组件整树、前端联合或生产验收；本次 runner 未构建、未触碰生产；该固定候选曾作为 0.1.19-3 受限影子 Core 安装，当前已前向至新960／0.1.19-4，正式旧 Go 未切换。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/run-8117d0a7ea1fe83d/RESULT.private.json`。

- **旧960 GitHub scope 修复（历史45项与v4安装保留）**：[`947557c`](https://github.com/TokenNotIncluded/lmm-api-source/commit/947557ceee21994ea97fc89afb3a84f86f783fec) 仅修改 external_login provider 与既有 authorization URL 测试两个文件，将 GitHub `read:user` 恢复为原已授权的 `user:email`；其余 provider、state、PKCE、redirect 与 subject ID 实现不改，Rust 产品单测仍 NOT_RUN。新固定 ELF `960196733396c24337eab69885ec2be14010aedfafd6a0fbdada3c1f3a3558f4` 构建退出 0（636 秒、OOM=false），12 个 source 输入无漂移；新 31 HTTP＋7 RP 配置实际 PASS/exit0，原 phase3 内同 ELF 软件实签 6 项实际 PASS，独立 scope-only v2 再实际 PASS1/TERMINAL exit0（0.575 秒），资格合计为 38＋6＋1＝45。scope-only 用 accepted_legal=false／双 legal=false，实际校验唯一 user:email、DB PKCE S256、Client/Redirect、state digest 与 binding cookie double-derive；accounts/users/oauth_subjects/credentials/families/proofs 前后相同，无 Subject/Session 创建、无外部 GitHub 请求。原 phase3 因 fixture 缺必填 bool 而 400/aggregate exit1、scope-only v1 HMAC fixture FAIL 均保留；不重跑 38/6，不复用旧 5bc44。final cleanup PASS，所属 Core/PG/runner 已停、container 已移除，固定 ELF/输入/全部 PASS-FAIL/原8材料保留；builder 未触生产、未新增 Cargo。该产物曾作为受限 0.1.19-4 影子 Core 安装，当前已前向至下述66ead／0.1.19-5，该旧gate不涵盖网站真实登录/RW/模型；Core5真实登录另按浏览器receipt记账，原8材料/硬件登录、真实 MFA 第二因子正例、慢请求体压力及跨进程并发撤销、CURRENT74 全树仍未运行。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/github-scope-only-v2-f057e676b3a83044/GITHUB-SCOPE-ONLY-RESULT.private.json`；`native69-github-scope-forward-1794698626-fbd978/{qualification-result.json,scope-only-v2-exit.json,final-cleanup-receipt.json}`。

- **GitHub authorization issuer 修复（新66ead／55门禁，受限Core5已装）**：[`48f4a7e`](https://github.com/TokenNotIncluded/lmm-api-source/commit/48f4a7e32f1a146f7dcdc466b910b9ad27f95d62) 精确四文件同时修复正常登录与 account-bind 的 present-iss 比较，使用共享 authorization_issuer；GitHub AS 为 https://github.com/login/oauth，原 identity/DB/flow/HMAC namespace https://github.com 及既有映射不改。Discord 必填、GitHub 缺 iss 的既有可选合同和其他 provider 语义保留，无 allow-both 或新 config。Current 与 sealed960 的 legacy_auth/external_login.rs 前像不同，按各自 exact patch 适配；原12＋四文件重叠provider一处，最终15个唯一输入，非CURRENT74整树。新固定 ELF `66ead3852418bc25c4562c20902676e60c469113a900494c9c55c3b29388e9a6` 实际重跑38项，另 e17＝6软件实签＋1scope＋10issuer全部PASS，TERMINAL actual_exit0／1.504秒，资格共55；旧96045全以新产物重验，不移签旧结果。normal/bind各5个issuer用例：wrong AS与旧identity AS返回401且对应flow保留；canonical AS＋固定é本地400，normal消费oauth flow、bind消费两flow；GitHub缺iss仍按旧合同本地400消费，Discord缺iss仍401保flow。issuer十项维持identity rows/subject/session计数、旧namespace/HMAC/实际S256；固定é在provider本地ASCII校验停止，runtime onlylo，无外部GitHub token/user请求或造Session，不证明真实GitHub登录。Rust UT仍NOT_RUN。qualification与final cleanup PASS，测试Core/PG已停、builder移除，15输入/固定ELF/原8/旧FAIL保留；标准Core0.1.19-5安装/restart实际0，运行PID202997、ready/status/setup200，Web3/Ext原服务与旧正式站不变。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/issuer-e17-bc3715b724cce174/ISSUER-E17-RESULT.private.json`；`native69-github-issuer-forward-1794703685-f3626e/{qualification-result.json,final-cleanup-receipt.json}`；`dual-origin-20261115/native-installation/core-66ead-installation-result.json`。

- **旧5bc RP 软件实签（历史另6项通过）**：同一 `5bc614a4` ELF 与固定 PASS fixture，实际软件 P256 签名使用 `https://native.api.lmm.best` Origin、`api.lmm.best` 父域 RP；UP-only／UV=false 与 UV control 均返回 200 并获得 NativeSession。sibling、错误端口、错误 scheme、错误 RP hash 四项分别返回 401，Session family、proof、session credential 均不增长，存储 Key 与 revision 不变。正例 counter 从 137→138→139，存储的原 UV=false、BE、BS 保留。`rp-signed-v2-exit.json` 为 TERMINAL、实际退出 0、1.272 秒；RP runner 的 Cargo 次数为 0，supervisor 的累计构建 1 次来自之前唯一 ELF，原元数据不改。主 31 HTTP＋7 RP 配置未重复，不读取原 8 份材料，实体硬件与真实 MFA 正例未运行，未操作生产。实签初版在 review 阶段因 CLI 问题拦截，未执行，不记运行 FAIL。证据：`fresh69-forward-preparation-26752cdaae25/auth-reconcile69/tests/rp-signed-f843f332547a58e9/RP-SIGNED-RESULT.private.json`；该 gate 最终 cleanup 通过，所属 Core/PG/runner 已停，builder container 已移除，固定 ELF、冻结输入、全部 PASS/FAIL 证据保留，不影响生产或其他 runner。

- **旧正式站单次模型调用**：一条真实临时 Key 请求返回 200，输入 15、输出 4 token，实际扣费 19 quota／0.000038 USD；临时 Key 932 已软删除读回，私密文件与自有剪贴板已清理。仅证明这一次请求与扣费完成，不代表全部模型恢复；未追加调用。证据：`dual-origin-20261115/old-model-singlecall.private.json`。

- **CURRENT74 按 scope 收口，整树验收尚未运行**：注册／联邦 11 文件已按 Main 授权以 [`7a00fb2`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7a00fb2) 单独提交推送，精确匹配 Raman 正式采用的 final，包含两处新增测试，未重复 apply。状态为 STATIC_ONLY，编译与业务全 UNRUN；manifest/base/工作树/final/暂存摘要逐一匹配，材料 `/tmp/lmm-current-registration-closure/`，不混 http 单位、GitHub provider、协议或客户端 fixture。`http.rs` 的 13 个权威 credit 单位字段及对应测试已由 Main 静态采用，以 [`1684ee3`](https://github.com/TokenNotIncluded/lmm-api-source/commit/1684ee3) 仅两个语义 hunks 提交；scale/exact 均 500000，无虚构 CNY 汇率；当前 Rust 与唯一 CURRENT 实际 `/status` 验证仍 NOT_RUN，已装 Native69 不含该补丁，两处 FED formatter 保持 dirty。协议 owner 已冻结 encode.rs、decode/zconv.rs、relay_http.rs 的 exact 三文件供唯一 CURRENT 测试，`FROZEN_FOR_CURRENT_TESTING` 不等于业务 READY/PASS；base db13602、patch SHA256 `8d643e3abe16a9c70aa782455f15924b85884b3b0a3528bddf82210b57c2e919`，CURRENT-base patch 无额外 Go lifecycle hunk，完整文件从 base 继承部分另行验收。材料 `/home/lightjunction/.cache/lmm-protocol-routing-20261115/current-threefile-freeze/` 含 manifest/patch/目标 matrix、vision 与后续负载命令；旧 v2 FAIL/build UNKNOWN 保留，最新修复业务 NOT_RUN。可准备唯一 CURRENT artifact，新 Native Web 窗口优先，不同时构建、不另开 Cargo；Native69 资格不外推至当前整树。

- **助手 history v3（源码已推送，限定范围独验通过）**：[`ed71cb1`](https://github.com/TokenNotIncluded/lmm-api-source/commit/ed71cb16f8e4d74310412b864451c5fc6ef12bc1) 接入 Conf 精确 13 文件，正式 history 保留 human、conversation 外键、sequence、nullable 实际作者及原空标题／preview／消息空壳列表语义；显式 owner CLI 负责升级、恢复与冻结 projection，runtime 无 raw stage 权限。testdata 为公共 v2 DDL，内联数据仅合成 9001/9002/9003；两真实隔离 PG、三 Go 包与 CLI/格式/文档自测通过。Volta 首次独验发现 missing-owner/NULL 反例，原 FAIL 保留；[`f4be2bc`](https://github.com/TokenNotIncluded/lmm-api-source/commit/f4be2bcdc2395ac1fa61a4a26df193174f4a9176) 仅补修两个文件，以 JSON 类型与 NULL-safe 比较拒绝必填身份字段缺失/null/错误类型，未知 actor 仍 nullable。18 边界及原反例、真实 CLI/owner API、replay/rollback/unknown-lineage/ACL 独立复验通过，实际退出 0、PG cleanup 0，其余 11 文件未变；原 no-tests、fixture owner 和独验 FAIL 均保留。证据：`lmm-completion-20261113/ahv3-independent-2d938704/fix/RESULT.json`。独验使用 `Go1.27.4-X:nodwarf5`，该次独验不抵扣项目正式工具链资格；后续仅 qualified639＋已发布13文件助手闭包的项目 Go1.27.2 scoped bin 单次构建实际0（5.612秒、sourceStatDrift空），不包含CURRENT90file。影子正规 owner CLI upgrade 首次实际1保留，后续 upgrade/project 各实际0；读回v3、805 conversations／2549 messages／41 human／51 hidden／754 visible，message/conversation全字段差异均0。原raw计数与指纹不变，142条support仍仅raw保留、正式support链未恢复；runtime LOGIN与原ACL已恢复且连接验证通过，越权privileges为0，模块保持OFF、所有既有服务不变。该升级/投影不代表已启用助手或整套Native/CLI业务验收，旧Go正式站未切。新增证据：`go-assistant-v3-native-inputs-1794701158-2afa2e/delivery-receipt.json`；`dual-origin-20261115/assistant-v3-live/result.json`。

- **OIDC token/userinfo 测试（Main 静态采用，PG 业务未运行）**：[`c65f578`](https://github.com/TokenNotIncluded/lmm-api-source/commit/c65f578) 仅提交 `oauth2_lmm_author.rs` 现有 test-only 增量（73 行新增、1 行替换），通过 issuer clone 保留引用，新增真实 router/token exchange 以及 GET/POST userinfo 的 no-store、仅 sub 断言，保留原 discovery/JWKS/bearer/disabled 断言。定向 rustfmt 与差异检查通过，实际 PostgreSQL/业务测试仍 NOT_RUN；Magpie budget、common fixture、注册闭包及其他 dirty 未夹入。

- **Magpie test-only scope 已分别推送，业务未运行**：Arch 明确认领的 with_magpie_fixture 四行 Arc/Send/static 签名与 recovery addendum 精确后像，以 [`6ad4279`](https://github.com/TokenNotIncluded/lmm-api-source/commit/6ad4279) 两文件收口；recovery SHA256 `1be6e2725d27458a50826a4c09f0cf9521e211e6f86fa25e190f84cbf4da5210` 匹配。Main 逐行采用的 budget 52 行 test-only diff 另以 [`db13602`](https://github.com/TokenNotIncluded/lmm-api-source/commit/db13602) 单文件提交，包含 owner 隔离 fixture 事务内临时停/复启 request_identity 并 assert O 的历史授权时间、developer 撤权拒绝/恢复、模型 dispatch 同 family/无 key_id/预算49断言；不冒真实 elapsed，不减少旧业务断言。全部实际 PG/模型业务 NOT_RUN；原实际 exit101、撤回排队重试与 recovery runtime UNKNOWN 保留。未纳入未知 docs/magpie-backend 或其他代码。

- **Native Web GitHub 严格 scope 窄修（已装，Core5实际登录另验通过）**：[`2031c38`](https://github.com/TokenNotIncluded/lmm-api-source/commit/2031c38d40b5f48e121c22cfa0a0197828d96423) 仅三个文件各一行，将 external-oauth.ts 的 GitHub preset 改为精确 user:email，两套 unit/hook fixture 使用独立字面量合同、不再跟随同 preset 生成。旧 scope 对新合同两套均实际 FAIL，修后 15 unit＋21 hook＝36 PASS，提交与 owner receipt 三文件摘要匹配；不放宽任意 scope、endpoint、Origin、Client/Redirect、state、PKCE S256 或 intent，无新 Core 改动。用户已授权以原72ef封印输入＋这三文件 overlay 在新目录进行一次 Native-only Web 构建/部署；现一次 build/bundle 均退出0，标准包 0.1.149-3 已交付（SHA256 `88479913315493b1619e8bd8871938e8b504fe172299b34152c311ab37c6e30c`），604个文件摘要及 compiled user:email 已核；这是 base+overlay 对应 Source patch，不冒充 whole HEAD。未全量 typecheck，原 base 三项 FAIL 保留；原 Native72ef/v2 产物、收据与真实 start200未导航反例保留。安装实际退出0，公网 index 摘要精确匹配 `de2b857089631a1d9eb43cd326c117d1a4f9d087d84e718c583ddb0041206956`；Core/Ext/旧Go PID 与旧正式 Web 链接保持，未重启服务。Core5新文档真实GitHub Root登录已实际通过；Users DTO待修、Resources独立route UNKNOWN，写操作与Native模型仍NOT_RUN，具体以浏览器readback为准。证据：`native-web-v2-72ef687-20261115/oauth-github-scope-fix/{fix.patch,receipt.json}`。

- **Core5 浏览器实际（真实Root登录PASS，页面/业务仍有未验范围）**：Terra 使用新文档正常新意图完成GitHub start200→callback API200→Root self200；history/stat均200，total/data_rows实际8行，系统信息Online、Ready=true。此前Core4 callback401与旧Web未导航反例保留；本次真实登录按独立浏览器回读记账，不从55门禁推导。Users接口200，但UI DTO形状不兼容待修；系统信息可读不证明单独Resources route，该route的HTTP证据仍UNKNOWN。写操作、真实付费Native模型调用仍NOT_RUN；不外推CURRENT74/全站生产已切。证据：`payment-restore-20261115/shadow-core5-browser-readback.json`。

- **Web 用量日志测试窄修（源码已推送）**：[`585e417`](https://github.com/TokenNotIncluded/lmm-api-source/commit/585e417) 只修一个测试文件的 sortOrder 字面量和新数组上的 reverse/find，21 项定向测试及修后唯一全量 typecheck 通过。没有更改 13 个 Web 产品文件、重建或回写旧 receipt；原 72ef 冻结构建的 typecheck 失败、build／bundle 通过记录保持原样。

- **原 Passkey 实际恢复（Native69 影子库）**：冻结源 8 条有效凭据、7 位原活跃用户已通过 NOLOGIN owner 守卫事务 COMMIT，独立事务完整读回匹配；原 credential ID、十进制 user handle、公钥 JSON、计数、UV 与 backup flags 保留，2 条 UV=false 和源 Preferred 策略不拔高。原空显示名按既有合同补标签；冲突不覆盖新登记。导入回执与审计各一条，未写 Session、密码、OAuth 或资金，未重启 Core。2315 兼容资格来自既有发布归档绑定；真实硬件登录未运行。私有证据仅存 `fresh69-forward-preparation-26752cdaae25/plans/passkey-recovery-20261115/RESULT.private.json`。

- **登录页留白改版（历史版本记录）**：已推送 [6ab724b7](https://github.com/TokenNotIncluded/lmm-api-source/commit/6ab724b7f78804cb67230e96f07779b1b156cb06)。登录表单采用居中 400px 布局、52px 控件与更大留白，底部小游戏按需展开；协议确认保留，重复说明移除。`user-auth-form.test.tsx` 47 项、`mobile-auth-presentation.test.ts` 1 项自测通过，均为 0 失败；改动范围 `oxlint` 与 `i18n:sync` 通过，locale 无差异。已在 1363×936 浅色桌面预览实际组件，测得表单 400px 且无横向溢出；已修正观察到的密码框与主按钮高度覆盖。后续浏览器连接超时，该历史版本当时未完成最终截图、移动端和深色复验。该历史版本的全量 Web 类型检查两次均被环境 `SIGKILL` 终止（退出码 137），没有通过结论；合并 `a7e2730` 后生产构建与首屏检查通过（gzip 594,658 字节，上限 786,432），移动布局 1 项与范围 lint 复跑通过；该历史版本当时未执行独立验收与部署。当前桌面、手机、明暗主题布局、类型检查及静态 Web 部署结果见本节 2026-11-17 更新，独立业务验收仍未执行。

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
| AUTH-01 | 部分验收 | 正常 OAuth 回调、claims 拒绝、同会话重启与 refresh 轮换已有真实 TLS 浏览器证据；WeChat 绑定解绑、绑定锁竞争及用量读取权限已有独立证据。旧回调不覆盖所选账号，以及在途取消原/新家族撤销均已通过独立浏览器与 SQL 核验。独立负例发现仅删除取消标记可复活旧流程，以及无 Web Locks 时迟到响应可清除所选账号 Cookie；已补修并通过定向自测与新冻结限定安全复验；冷双标签页正常及取消仍通过，首次夹具失败保留。窄屏提示遮挡已补修并通过新构建七语言独立视觉复验。当前恢复提示、崩溃 pending 晋升及显式重新登录已通过受控 HTTP 的浏览器开发自测；当前影子匿名无 Cookie 恢复已通过实际浏览器回读；真实已有账号与 Core 的联合恢复、过期旧凭证且刷新 Cookie 已丢失、旧 provider 配置迁移与正式配置仍待验，Google 回调 URI 阻塞见当前状态。 |
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
| 管理员用户原生用量开发自测 | 开发机 /tmp/lmm-architecture-20261116-jpmxei3q/：user-usage-core-final、web-targeted、web-static、core-clippy-final、web-format-final、web-build 的日志与 result.json；截图及 8 组报告位于父仓库 .git/lmm-architecture-user-usage-v3/，仅合成场景，不替代独立整链路验收 |
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

# 新架构进度

这是源码仓库与配方仓库共用的唯一进度列表。2026-11-08 重新建立，按产品行为与验收结果分组；旧任务编号、旧勾选状态和历史完成率不再沿用。架构规则见[核心与扩展](core-migration.md)，完整验收范围见[生产验收合同][acceptance]。

## 本地收尾（2026-11-12）

- `6991df0` 增加既有订阅套餐的 owner 价格发布入口，复用正常付款配置校验和双价格、操作回执事务。`0510c24` 追加第 67 组件，保留省略 checkout language 的 provider 推断语义。Core4 正式本地 ELF `fb05a273…0d661` 已通过正常 67 初始化、七个实际 ACL 拒绝、旧真实 Core66 ELF 正常初始化后的升级与重复执行，以及九组 SDK 付款集成检查。新增空语言请求实际省略 SDK language 字段。旧升级 fixture 的两个前置错误及日志保留，前后源码与 147 SDK 字节均核对一致。
- `f5667e2` 支持严格的未准入 67 组件捕获。`3ebb537` 的正式 67 工具准入绑定实际初始化、ACL 收据。`667f35a` 的实际 PostgreSQL 回归共有十四个唯一用例通过。首轮 W 用例在身份导入前置的其他数据库客户端检查被拒，W 业务尚未执行。独立重跑 W 通过，原失败结果保留。
- `a9c1210` 的退款来源与账户映射修复通过十一项检查。`9de5880` 的连接期限和回执投影通过一项传输单元检查、三项真实 PostgreSQL 检查，完整表和序列 CAS 保留。`92c8443`、`46a0195` 修复 Originals 正常 CLI 的 Decimal 往返、私有文件及输出容量，十八项定向检查通过。上述 W 实际 SQL 回归另证明 UNKNOWN 原值、整数型 Decimal、重复执行和冲突拒绝。
- `7338dee` 的签到 Web 接线保留旧 Go 响应与 Native 精确整数、服务端日历和丢失确认后的同操作重试。`1f1e419` 补齐库存功能配置的真实入口测试与商店缓存修复。`907197b` 为同批库存添加保留稳定 request key，覆盖未知确认后的刷新重试。固定 Web5 全部 737 个源码测试文件和 7 个脚本测试文件、类型检查、相关 lint/format、构建及 bundle 检查通过，源码测试执行 5066 项、脚本执行 47 项。输入前后字节相同，distribution、Arch 和 DEB 的全部载荷已校验。旧版本失败及 3 GiB 限额下类型检查 OOM 证据保留。
- `b62eeef` 补齐签到 Core、HTTP、owner CLI 和运行角色权限，七项定向纯检查及十一项真实数据库检查通过。历史 seed 恢复日标记，奖励保留 opening 原值。首次数据库检查发现 opening 查询的字符串引号错误，修复后整批重跑通过，失败日志保留。第 68 组件实际隔离捕获为 manifest `ae3dc3f4…20c12`、catalog `e3ca0200…16c02`，共 288 张表、36 个序列，153 项捕获输入前后相同，二十一项谱系检查通过。固定 UTC+08 日历由旧 fixture 全部 5986 条记录推得。旧 fixture 仅用于本地演练。
- Core5 正式本地 ELF `148729e8…831cc` 通过正常 68 初始化、十七个实际 ACL 拒绝及受限运行角色读写检查、九组付款 SDK 集成、Arch/DEB 正常安装升级卸载。配置、秘密、数据保留及运行中旧进程未被安装脚本重启均有实证。996 项源码及 147 SDK 包装前后字节相同。观察的 Core、Go 单进程 VmHWM 分别约 57.20、31.28 MiB，整套 Core/Go/PG/Python cgroup 峰值约 279.09 MiB，口径分别保留。目标生产主机 ABI 和生产启动继续验收。
- `4fb75d5` 为 Passkey 迁移加入正式来源映射守卫，`422e462` 保留捕获源的用户验证策略。`99b0aff` 修复旧 Go Preferred 策略的真实 UP-only 签名被新版拒绝的问题。旧 Go 同一断言接受、旧 Core 拒绝及修复后的正常登录均有独立实证。开发二进制的十七项真实签名矩阵通过，Native Required、错误签名、RP、origin、handle、计数器、重放、BE/BS 拒绝规则保留。Core6 正式本地 ELF `789336d0…ff164` 另通过正常初始化、ACL、十一项实际签名检查，以及指定 Go9 的九组 SDK 检查，正常制品与开发内核证据分别保存。
- `76aaf71` 接通密封库存的正常 Go HTTP 路径并补齐付款、交付前的密钥条件，六并发抢两件实际仅两次成功。固定 Core6 `99b0aff`、Go9 `76aaf71`、Web5 `907197b` 的独立十五项 HTTP 钱链通过，包含真实正常登录、模拟渠道签名人民币充值、购买、付款、提货、退款、双方资金守恒、重放及库存不回补。另十七项同源浏览器组合通过，覆盖双用户登录、服务器已接纳但客户端丢失确认后刷新同 key 仅入库两件、人民币 checkout 恢复、597 个静态文件及全部 89 条静态路由。外网实际付款和生产部署另行验收。
- `ebf6673` 的正常 68 迁移工具准入绑定 Core5 实际初始化和 ACL 收据。`432ec3b` 的三十五个唯一真实数据库用例跨两轮通过，首轮新 ACL 测试的列名错误及修复证据保留。W 实际 SQL 另覆盖 18 个 bound pending、19 个 durable UNKNOWN，以及空语言、Decimal 原值和重复执行。
- 旧 `412384d3…` 源快照的全量本地迁移演练已核对 213 张源表，财务子集实际导出 140 张表、256,425 行，正常身份导入 1247 人提交及读回通过。实际源数据有 8 个有效 Passkey、192 个待处理充值候选、45 个订阅 pending、50 项库存。启用 MFA 为 0，另有两条停用记录。此次演练使用独立本机目标，旧 epoch 继续保留非部署候选标记，未来切换仍须重新捕获停写快照。
- 本地资金首个 attempt 因自设 1 GiB、swap0 限额 OOM 而断连，原 UNKNOWN 保留。同一数据目录以 Max4 GiB、High3 GiB、swap4 GiB 正常恢复后，默认严格解析仍因启动时间变化保留 UNKNOWN。`0ab36c6` 的正式重启只读入口通过七项单测和一项双数据库真实重启用例，核验原材料、SQL、审批、工具 hashes 及连续性证据后，实际确认 NOT_APPLIED。随后同 migration ID 正常重准备、绑定既有授权，新材料 `13d06ebb…09ba0` 实际 COMMITTED。1247 份钱包和历史计数、账户映射及全部源值读回相同，总账合计为零。每 500000 原单位等于一美元，钱包正额 USD 1540.576994，4 份欠额合计 USD -0.213238，钱包净额 USD 1540.363756，40 份带符号推广余额净额 USD -67.815478，5 份暂停托管、12 条 claims 的剩余托管额 USD 106.804952。私有证据保留在 `auth-continuity-rehearsal-8hqzzqo8` 的 `opening-applied-local-01`、`opening-restart-resolved-local-01`、`opening-applied-local-02`、`opening-actual-readback-local-01` 和 `opening-recovered-attempt-lineage.private.json`。原 UNKNOWN、NOT_APPLIED 和新 COMMITTED 的材料、SQL、审批与 attempt 链全部保留。生产迁移与切流继续待执行。


- 同一旧快照和正式身份映射下，plans 正常 preview/apply 及实际读回通过，7 个原计划和 45 条 pending liability 已落库。首个私有读回误用 plans migration ID 过滤 pending，修正为正常合同绑定的 opening migration ID 后通过，错误 SQL 与实际结果保留。S15 由正常 frozen-export mapper 重新生成无日历时区的 plan，15 条已过期、`never` 周期权益通过既有 history-only 合同。正常材料 `d076280b…1eea0` 实际 COMMITTED；独立读回 15 条 inactive grant、15 条原用量和逐项 owner 映射全部相等。原资金材料的 UTC 绑定保留在原证据链。证据为共同演练目录的 `subscription-prepared-local-01`、`subscription-applied-local-01` 和 `originals-financial-attestation-local-01/subscription-readback.receipt.private.json`。
- S 提交后重新采集实际目标快照、生成正常 F attestation，再经正式 W CLI 准备和既有本地授权执行。原 SQL `648cd2c5…16347`、材料 `bd4af506…a3ebf4` 实际 COMMITTED；正常 SQL 的成功返回及独立持久回执七项核对通过。最终独立读回 60 条命令全部 UNKNOWN，逐条结果、个人账户归属和 7 条既有 S 历史链接相等，S 的 15 条 inactive grant 与用量记录相等，外部及 Pancake 价格、订单各为 0。53 条普通 UNKNOWN 中，balance 成功 8 条缺原 snapshot 与扣款材料，epay pending 8 条缺 snapshot、现金金额与币种，Pancake pending 19 条另缺产品和商户材料，Pancake pending 18 条由旧 overflow 政策触发兼容缺口。另 7 条 Pancake 成功同因 overflow 政策保留 historical-only。45 条 pending 继续保留潜在迟到付款责任。原成功事实、冻结材料与当前钱包值归档保留，缺失材料须从原始扣款、冻结价格或提供方持久付款凭证补齐，overflow 项须由匹配原政策的兼容路径恢复。UNKNOWN 继续等待恢复处理。私有分类及恢复说明位于 `full412-go-producers-84jgu6ps/subscription-originals-source-01/unknown-rights-classification.private.json` 和 `unknown-rights-recovery.md`，实际证据位于共同演练目录的 `originals-prepared-local-01`、`originals-applied-local-01`、`originals-readback-local-01`。Astra 的 W60 只读静态审查与本轮实际提交、读回分别留档。Core 唯一写入窗口已交接 Pascal 继续凭据阶段，凭据验收仍待完成。以上均为旧 epoch 的本地演练，未来生产切换仍须新停写快照及实际迁移、切流验收。

## 当前线上状态（2026-11-11 回切旧服务）

- 按用户要求恢复旧服务。Arch 与 Ubuntu 的旧 Go 服务均已恢复 active，两个 Nginx 入口均恢复 legacy-active；公开首页与 `/api/status` 实际返回 200，状态接口为 success、版本 `0.2.118`。Ubuntu 首次启动超时，后续自动重试已恢复运行。
- 本轮停写期间已实际导入 1247 个身份、钱包与欠额、带符号推广余额、五笔托管、商店与支持数据，以及七个订阅套餐和待付记录。正式历史订阅权益、剩余凭据与 OAuth 恢复、实际启动验收及公开切流仍有工作。原生服务保持离线，首发尚未完成。
- 旧服务重新接受写入后，`FINAL_20261111T120751Z` 及其派生候选作废为未来切换输入；原始证据保留。后续切换需要重新捕获源数据并重新绑定目标状态。
- 回切前 Core `63e0540` 与 Go `8c52178` 的隔离组合已通过原八组及新增人民币、禁止余额购买订阅的检查，Web `a593abb` 的二十项定向检查、类型检查与原生包安装通过。后续本地修复和验收见上，完整迁移与发布流程继续隔离验证。

## 当前生产首发范围（2026-11-10 用户调整）

- 普通 HTTP 请求的终态恢复已接入。恢复时按原冻结价格重新归一化供应商用量，并与已持久化计费检查点核对。隔离 PostgreSQL 的 4 项定向测试通过，包括审计记录单独存在、用量不一致、成功重扫以及流式和 WebSocket 回归。
- 当前账本离线预览已扫描财务范围的 140 张表、255,524 行，完整源表清单包含 213 张表。五笔托管按暂停合同保留，既有六项谱系问题已补齐。三笔物理缺失用户的历史充值仍保留原始证据和 UNKNOWN 状态。最终停写快照、全部导入阶段和实际目标读回仍待完成。

首发目标调整为现有生产站点实际在用的业务可上线、可用；未使用功能和非关键问题允许延期，不再要求下方 39 项全部关闭后才发布。完整功能清单保留为后续工作，延期不等于已实现或已验收。 用户随后明确商店和订阅也必须首发可用：现有购买、库存提货、退款和订阅权益同属必要范围，不能以延期其他扩展为由移除。 Gemini 音频输入经用户确认未使用，允许延期；文字和图片调用仍须保留，不能将音频按普通文字价格计费。

首发范围以生产只读核对的启用配置、实际调用与已有业务数据为依据，不能仅凭存在路由或功能名称推定使用情况。当前在用的登录、模型调用、计费余额、数据和持续权益迁移、权限隔离及切换回滚必须通过固定候选的相应验收。未启用的扩展应保持关闭，明确列出延期功能和已知限制；不为未用功能继续扩展数据库合同或阻塞首发。发布制品、部署和真实运行结果分别记录。

## 首发阻塞与已验证候选（2026-11-11）

- 正式 Core `184569d` 的二进制 `48ca3bd7…4a05b6` 与 Go `8c52178` 的 `d7c4260e…f9e856` 已通过最终原八组、UNKNOWN 第九组及双币第十组真实组合检查。原八组在一 CPU、一 GiB、swap0 下峰值 378544128 字节，无 OOM；第十组经正常 SDK、独立签名和真实 Core 验签，美元 198 分与人民币 1327 分各入账 1000000 单位，并验证冻结汇率、重复通知及拒绝错误报价和签名。
- Arch 已安装正式 Core Native `0.1.19-2`，实际二进制哈希及七项运行配置核对通过。只对本次新建的空 `lmm_core_prod` 执行旧 65 组件全表、种子、序列同实例 CAS 后重建，正常 Core 66 `init-db`、八项能力的 659 项权限核对、基础及实际服务账户完整 `check-db` 全部通过。账号与资金仍待最终停写快照迁移，Native 服务尚未启动，公开流量仍为旧服务。
- 首发所需 Go payments/store 已正常安装 schema 与 canonical grants，真实 runtime SCRAM 登录及跨库隔离检查通过，最小运行配置和原有两篇中文法律文档已安装。未启用工具市场的安装错误保留并按首发范围延期。zcall 原生 `base_url` 接入依用户指示列入下一版本。
- 人民币新充值纳入首发，先固定到账额度，再按原汇率与八档套餐折扣计算实付金额。四项实际 PostgreSQL 检查通过，覆盖美元与人民币报价、冻结汇率重放、独立签名入账、累计退款与末笔预留、报价约束和防篡改；912 项源码输入前后未变，隔离数据库已停止。完整配置读取曾因 untagged 配置缓冲字符串键而拒绝折扣表，严格键转换修复后完整配置检查通过，两项报价退款数学检查通过。新数据库合同追加第 66 个组件，完整目录已在本机隔离 PostgreSQL 实际捕获，并经 Astra 只读复核，正式 Core 正常初始化与生产目录核对仍待执行。
- 人民币候选 `8c52178` 的 Go 和 Web Native release 2 已安装到 Arch 主机，Go 实际二进制及 Web 597 项文件逐一核对通过。两台 Nginx 已完成站点模式预装并 reload，实际 TLS 状态读取均为旧 Go `0.2.118`，原二进制哈希保持。首个 Core 66 构建用于捕获，因内置旧 65 组件 pin 而无部署资格；本次将 fresh pin 更新为实际 66 组件 manifest 与目录哈希，随后重建正式 Core。
- 迁移工具的真实 SCRAM 回环传输、组件独立身份与商店导入修复已提交推送 `fd56ccf`。当前九项输入在固定 `c0d259a` 的正式 65 组件来源上通过 12 项纯检查和实际 SCRAM PostgreSQL 回归；第 66 组件目标的回归等待新制品与正式目录指纹。
- 固定 Core `a69458e…a3d6f` 与 Go `c0d259a` 的原八组业务联测及第九项 UNKNOWN 恢复检查均已通过，未决原始证据、重启与重复尝试保留正确。该结果属于人民币改动前的固定候选，新组合仍须重新验收。生产资金与账号尚未迁移，公开流量仍由旧服务承担。
- 已在 Arch 主服务器安装 Core 与 Web 的 `0.1.19` Native 包，Core 实际文件哈希匹配 `a69458e…a3d6f`。九个独立 `_prod` 数据库及 27 个隔离角色已创建，运行角色跨应用库 CONNECT 为零，原有五个数据库保留。实际 Core `init-db` 成功，尚未导入账号或资金，Core 服务保持 disabled/dead，原 Go `0.2.118` 继续服务。原模板一因系统 collation 版本变化拒绝建库，改用 PostgreSQL 原生 `template0` 创建新库，原库与模板未刷新。
- 用户已确认最终停写快照按每 500000 单位等于一美元原值迁移，保留欠额、带符号推广余额和全部既有托管权利。审批记录已经保存，最终源哈希绑定和资金导入尚未执行。
- 修复用户 ID 被误当个人账户 ID 的迁移错误，正式身份映射用于 Key、订阅、钱包、托管和历史引用，纯用户字段继续保留原 ID。22 项单元检查及 7 项实际 PostgreSQL 检查通过，覆盖非同值、数字碰撞、重放与失败回滚，源码 `ff992f8` 已推送。
- 固定 Go `26162a4` 的第九项实际恢复检查发现 pending 结果写入空 bytea，触发约束回滚，未决证据的尝试次数未递增。SQL NULL 修复已通过四项实际 PostgreSQL 回归，商店与 payments 两包的 501 项检查通过，其中 PostgreSQL 检查另行提供环境执行。须重建 Go 后重跑原八组及第九组。商店与客服的正常安装入口已接入现有 schema，通过实际受限登录、CRUD、跨库拒绝及正常 composition 启动检查。导入 SQL 也已在真实 PostgreSQL 执行，覆盖账户碰撞、退款库存与请求角色保留、客服隔离、回滚及重放拒绝，待纳入新 Go 制品。
- 最终 Core `6283afc` 可执行文件 SHA-256 `a69458e7729a01b92f1142d0c6d69ef6dbf53fc22e1fc39b2cd56cd54e2a3d6f` 已构建，实际两机 308 项动态符号及版本核对通过。固定 Go `26162a4` 的原八组真实业务联测全部通过，一 CPU/1 GiB/swap0，完整拓扑峰值 347,652,096 字节（331.55 MiB）、零 OOM，979 项冻结输入前后无漂移。回执 `~/.cache/lmm-subscription-6283-rsa-fgmpn2lr/acceptance-receipt.private.json`，正式 Core preview 分发及 Arch/DEB 包已验证。Go 保留原 `0.1.0-preview.20261111.26162a4` 身份，Native 版本转换支持该完整标识，22 项包装检查通过。实际历史迁移和生产切换继续进行。
- Native 包装工具已显式识别 OpenSSL 3 的 SONAME，并从实际 ELF 符号版本提取最低依赖版本，Arch 使用真实 SONAME provider、DEB 使用 libssl3t64，OpenSSL 4 继续由未知依赖保护拒绝。21 项包装检查通过。Core 制品源码固定 `6283afc`，包装工具修订单独记录；最终正式 ELF、分发归档与两种本地包验证继续进行。
- image2 原源码的缓存字段全缺省行为已通过显式冻结合同接通，默认 Native strict 和旧已存报价保持原义，正缓存缺分类仍保留未决。269 项 Core 定向及集成检查通过、1 项私有旧 VM 检查跳过，677 项输入无漂移；真实 HTTP/PG 验证旧报价恢复、原始 usage 留存、正常计费与拒绝注入。证据 `~/.cache/lmm-image-source-api-noidea.6ocu0v4y/summary.private.json`。另一次扩展绘图检查保留一项旧 Cookie 数量断言失败，两相关文件与 `114e181` 字节一致，现有身份专门测试明确双路径各设 Cookie；该检查未纳入本轮图片 API 通过范围。
- 供应商输出元数据保留完整上界，实际请求继续取现有安装限额与供应商上界的较小值；已移除两处将 128K 元数据误拒的额外限制。完整路由映射脚本 55 项检查通过，OpenSSL 3/目标 IDEA 排除配置下 214 项 Core 定向及集成检查通过，1 项私有旧 VM 检查跳过，677 项输入无漂移。证据 `~/.cache/lmm-provider-bound-noidea.ojwqtusq/summary.private.json`。此前 NoIDEA debug 对两台生产主机的 1,011 项动态依赖符号比对通过，正式 release 的独立 ABI 核对待完成。
- 模型旧预扣新增显式派生 Native 实际请求上界模式，Core 在同一事务按实际 trust 复核，保持已有静态合同。208 项本地检查通过、1 项旧 VM 私有夹具检查跳过，675 项输入零漂移；覆盖真实 PG 的 trust 并发变更、伪造 cap 拒绝、结算重放与崩溃恢复，以及账单、渠道统计、仪表盘和商店交易。已用 OpenSSL 3 SDK 和目标 IDEA 排除配置构建并测试；最终正式制品及两机 ABI 复验待完成。证据 `~/.cache/lmm-native-bound-noidea.ljwf2a2v/summary.private.json`，两次新增测试夹具错误及失败记录保留。
- 现场接线检查发现两项待完成。`c8719a2` 本机构建产物依赖 OpenSSL 4，而两台生产主机实际仅提供 OpenSSL 3，需使用匹配的完整 SDK 重建并复验。冻结模型配置尚未生成 `legacy_precharge` 合同，派生 Native 上界的源码已通过检查，生产模型配置装配和正常入口验证继续进行。前端已从 `efd750b` 准确冻结重建并完成 preview 分发与 Arch/DEB 包验证，版本 0.1.19；证据 `~/.cache/lmm-web-efd750b-freeze-_nyb0rin/packaging-receipt.private.json`。此前业务联测结果保留，生产候选更新后重新绑定同一组合。
- 新账本结构的独立 fresh profile 已完成实际初始化、完整种子和运行权限回执绑定。281 张表、36 个序列的基线通过核对，五项实际越权操作均以 42501 拒绝，授权及探针前后目录、数据和序列一致；9 项定向脚本测试及 24 项实际 PostgreSQL importer 检查通过，包含重放、中断恢复和混搭拒绝。固定的 schema 来源为 `abd532d`，后续恢复读取修复继承同一物理 schema。实际历史资金导入与生产切换待完成。证据 `~/.local/state/lmm-api/actual-init-acl.ZPAWetRJ`、`~/.local/state/lmm-api/current-ledger-import-pg.97u3yxoi`。
- `abd532d` 的正式 release 联测通过初始化、目录和运行权限检查后，在实际启动恢复扫描时报 `output_checkpoints` 读取权限错误，后续七组业务未运行。已将查询改为既有的受限 EXECUTE 读取接口，新增真实运行角色的启动、恢复、拒绝直接 SELECT 及第 1,001 项分页回归。最新 138 项定向测试通过、1 项旧 Go VM 私有夹具测试跳过，665 项输入前后无漂移。修复后的 `c8719a2` 正式 release 与固定 Go `26162a4` 已通过原八组真实隔离 PG/RSA/HTTP 业务断言，受限角色实际启动成功，购买后实际消费 34、剩余 966，钱包快照保持一致，重启与续期重放通过。一 CPU、1 GiB、禁用 swap 的完整拓扑峰值 286,154,752 字节（272.90 MiB），零 OOM，所有固定输入前后哈希一致，PG 与子进程清理通过。Core 可执行文件 SHA-256 `e7742d43f9e9bded72337718fa9599f940b282fb34bf0aaee0f7b5c1c3599b51`，通过回执 `~/.cache/lmm-subscription-c871-rsa-a7cexn3o/acceptance-receipt.private.json`。实际历史资金迁移与生产切换待完成。失败证据 `~/.cache/lmm-subscription-abd-rsa-8fq2gh55/acceptance-failure-receipt.private.json`，修复自测 `~/.cache/lmm-checkpoint-runtime-fix.emagI8KX`。
- 旧版纯文本预扣、原钱包补扣、表达式终态旧浮点舍入和崩溃恢复已接通。137 项本地定向测试与 31 项账单、渠道统计、仪表盘集成测试通过，1 项旧 Go VM 私有夹具测试跳过；665 项源码输入前后无漂移。真实 PostgreSQL 覆盖预算、订阅顺序、债务、退款、充值还款、HTTP、SSE 恢复及 TCP 提交回执丢失；HTTP 反例验证旧浮点 124 与原生精确规则 125 的差异贯通账本和恢复。装配时后续 SQL 覆盖账本动作的问题已修复，实际新目录摘要为 `d83aa95031f13c052adf1cc2344db0601b7932c3abc792b57683cb7cacf5b27d`。当前旧浮点分支限于明确冻结合同的钱包来源，历史导入订阅的旧舍入兼容仍待验证。实际迁移、发布与生产切换待完成。证据 `~/.cache/lmm-legacy-wallet-float.bBOUK4Ai`、`~/.cache/lmm-legacy-wallet-readers.6knLAl0M`。
- 旧托管候选新增有界的暂停项目来源合同，保护 parent 已记录字段并绑定完整当前行，新增 accepted claim 限于暂停前且尚未提交或付款。独立复跑 28 项单测通过；现有真实缓存的纯 mapper 验证在绑定新附件后解除 6 项来源阻断，缺附件继续拒绝。实际部署来源、历史单位权威和独立经济批准仍待验证；完整 CLI 重跑、实际导入及生产切换待完成。证据 `~/.cache/lmm-financial-rebind-zmvsaw34/paused-local-aggregate.private.json`。
- 对账验收的可到账孤儿充值统计已覆盖 pending 和超时失败两类订单，复用导入器支付生命周期规则。独立编码任务的开发自测包含 5 项隔离 PostgreSQL 行为测试和 9 项原有单测，均通过。测试覆盖漏 owner、正常 owner、普通失败和混合订单，未连接生产。证据 `~/.local/state/lmm-api/payable-orphan-tests/run-pze66l4p`。
- 财务预览复用已有支付生命周期规则，`waffo_pancake` 的 `checkout_timeout` 失败订单仍可后续到账，按未终态保留。映射工具证据加入该依赖文件摘要，109 项定向 Python 自测通过。孤儿 owner 阻断及未决退款权限继续保留，真实冻结快照的新版预览待重跑。
- 实报成本的网关终态统一保存经验证的收据并执行事务内结算，未超额请求也支持已进入 `reconcile` 的迟到终态。工作树组合 6 项隔离 PostgreSQL 定向自测通过，包含实际网关、真实钱包、HTTP 恢复及普通 OpenAI 回归。网关夹具的前两次失败分别来自外部预算合同要求和路由模型映射限制，修正为已支持的本地合成供应商及原模型后通过，失败日志保留。外部供应商实报成本仍要求供应商强制预算合同。证据 `~/.cache/lmm-cost-terminal-model.BHqGQ1zl/run.log`。
- 实报成本终态结算支持原 worker 在租约过期或进入 `reconcile` 后重新核验当前凭据并补额。工作树组合的 5 项隔离 PostgreSQL 定向自测通过，覆盖真实钱包充值后补扣、不同 Key 拒绝、回滚、退款、团队预算和订阅周期约束。首次测试因过期夹具漏增 revision 被数据库拒绝，改用已有 helper 后复跑通过，失败日志保留。无凭据后台自动补扣仍待实现，未生产部署。证据 `~/.cache/lmm-measured-recovery-fixed.DfW3udJu/run.log`。
- 新增 `scripts/export_legacy_key_target_observation.py`，从已提交凭据映射所绑定的隔离目标库只读导出原生 Key、付款方、额度与设置。9 项开发自测通过，包含实际 PostgreSQL 的完整入口、私有文件权限、漏项拒绝和读回前后数据库状态一致。输出保留原生事实，未生成旧格式对账清单，未验证批准材料摘要，也未连接生产。证据 `/tmp/lmm-key-observation-entrypoint-test.log`。
- 实报上游成本的补额与结算在同一数据库事务内完成，源码 `10c4864`。随后 `156dac2` 在隔离 PostgreSQL 的两项定向测试通过，覆盖注入结算失败后的补额回滚，以及真实钱包余额不足、充值后原租约内重试、结算重放和全额退款。这两项测试使用夹具预置的终态收据，网关终态写入已有此前独立用例的验证。旧钱包预扣与后台补扣的后续组合结果见本节最新记录。证据 `~/.cache/lmm-measured-wallet.kOsCja4s/run.log`。
- 目标级客户端协议限制已接入路由、重试和模型目录，普通渠道编辑保留已发布的限制。离线迁移接收明确审核的列表，拒绝空值、重复与未知协议。工作树组合的 6 项定向 Core 测试及 54 项离线迁移测试通过。更广路由回归先暴露 6 项缺少测试库和 1 项夹具请求未定价的快速档位，在隔离 PostgreSQL 18.6 重跑已通过 38 项、忽略 1 项，修正后的档位夹具待重编译复验。尚未安装生产路由配置。
- **尚未生产切换。** 主用付费文本模型已补显式绑定旧版本的预扣与权威实际用量结算兼容，原预算和订阅来源顺序约束已经定向验证。实际生产模型配置、同一冻结候选的数据迁移及运行验收仍待完成。
- 旧版预扣已保留请求原始输出上限。OpenAI 普通文本计数使用固定 Go v0.6.2 的切分规则和匹配的词表，两个编码器与旧 Go 对照的 20,014 个计数一致。Responses 纯文本提取保留源工具 JSON 和可见推理摘要，5 组请求与固定 Go DTO 的计数一致。普通倍率已从已解析的请求算出旧版预扣金额，省略和显式指定输出上限两组结果与 Go 一致，Core 的 10 项预扣定向测试通过。媒体和其他协议的旧版预扣兼容仍待接入，钱包事务增量见本节最新记录。
- WebSocket 可由模型配置 `responses_websocket = true` 启用标准 `/v1/responses` Upgrade，默认关闭。冻结工作树组合 18 项测试通过，覆盖原生路由、取消归属、轮换凭据后拒绝且不预留、两轮续接与终态先扣费、断线后实际终态结算或保留未知用量待对账，以及原 HTTP 请求计费。隔离 Nginx 的 22 项检查通过，包含实际帧往返。尚未验证生产配置、重启恢复与真实供应商，未部署。旧钱包无时钟表达式的浮点兼容见本节最新记录，结算时分时窗口仍待兼容。
- 路由可用组迁移修复 `464e5eb` 经冻结输入的 82 项 Python 测试通过；不扩大原 Key 的显式组权限。Root 激活后凭据导入绑定原生已消费审批、审计及策略事实，修复 `06b8661`、测试错误码同步 `26162a4`；helper/API Key/writer 三个完整套件共 82 项冻结复跑通过。这些是合成数据脚本检查，没有实际审批、导入或生产写入。console 的 A2 基础身份合同与当前组件装配联接、资金导入后的精确回执及初始种子验证已修复；冻结旧 schema 锚加当前两脚本的混合输入完整回归发现 66 项，其中 41 项通过、25 项 PG 检查跳过。实际数据库分阶段导入仍未验收。
- Core `26eb4aeb` 的正式 release 可执行文件已构建，SHA-256 `c0b4cdb89c9519623278525b563f7edafb18d7b71b2e04879145083efe0bd79f`；真实 Native65 初始化、目录/owner 检查，以及 Fable 12 项、旧整数价格 5 项 HTTP 运行通过。它未包含上述待实现的预扣、旧分时结算及 WebSocket 链，不能作为最终上线候选。
- 支付回跳地址为空的来源行为已由 `553982e` 保留；非空仍验证安全 URL，定向三项测试通过。源码 `26162a4` 的新 Go release 与分发归档实际构建、验证通过，冻结的 5197 个源码文件前后无漂移。可执行文件 SHA-256 `476077f1793c1811b6785b44fef867785095f5bfcf82dcd7a1aec58a0cd9bc16`，归档 SHA-256 `10884ccf86b3a7a5d258aade79e114dde0b3c701fb51c87a007d295417da4a42`；最新 Core 组合的八组验收见本节，发布与安装待完成。
- 商店与订阅仍属必要首发范围。固定 Core `26eb4aeb` 加新 Go `26162a4` 的真实隔离 PG/RSA/HTTP 八组联测已通过：购买 grant 后由模型实际消费 34 积分、剩余 966；回调重放、Go 重启及续期重放不重复 grant 或重置用量，17 张钱包权威表快照保持一致。整个 Core/Go/PG/合成供应商拓扑在一 CPU、1 GiB 内存上限、禁用 swap 的隔离环境运行，实测内存峰值约 303 MiB，零 OOM；此前缺少 Key 组设置、错误假设上游参数名及夹具 USD/credits 单位错误的三次失败均保留。这是此前固定组合的合成业务验收。含预扣与恢复修复的 `c8719a2` 组合复验见本节；历史迁移、逐笔对账、浏览器、服务器预算及生产回环仍待完成。

## 首发收尾记录（2026-11-10）

- 图片价格页和绘图入口已支持按实际 token 计费，单次请求预留不按图片张数重复计算；固定单张价格路径保留。源码 `b5e5aec`，40 项针对性测试、全量 Web 类型检查和翻译同步通过；浏览器及新 Core 联调未运行。
- 原生产模型路由与价格迁移补充 OpenRouter、xAI 的明确协议范围和 SystemOne，保留原计费模式、精确阶梯表达式及图片配置来源。源码 `3cc07d9`，34 项 Python 测试通过；不是实际生产迁移或原生运行验收。
- 旧商店库存与受保护提货链接迁移已补齐，保留库存归属、原买家、bcrypt 提货码、旧链接和退款状态。源码 `1f3cc90`，7 项 Python 测试通过；完整 HEAD 加这八个文件的独立源码快照定向 Go 测试通过，未夹带部分退款草稿。实际订单数据库迁移、旧已付款订单退款桥和整体验收仍待完成。


- 分时价格及明确声明非音频的旧表达式迁移已提交 `317d151`，38 项 Python 测试通过。Core 日历价格实现已进入完整库类型检查通过的组合；重试与结算的实际运行仍待验证。7 个非音频表达式已按生产只读渠道绑定唯一匹配，尚未安装运行配置。
- Google 旧 OAuth 绑定严格导入已提交 `14265b4`，26 项 Python 测试及隔离 PostgreSQL 合成数据导入、重放、冲突回滚和受限读取验证通过。验证使用旧 Core59 基线；最终 Core65 迁移及真实登录尚未验收。
- 原生图片回执的增量 SQL 和回归用例已提交 `3c8a673`。隔离 PostgreSQL 使用固定历史 identity/charging/image DDL 加新增 SQL，实际验证旧与新 JSON、未知缓存、非法字段、两处 CHECK 及受限角色逐函数撤权/恢复，通过后停止并清理临时数据库。首次旧约束名错误及第二次等待超时保留；第三次通过。尚未验证完整 Core65 初始化、HTTP 图片调用或资金结算。
- 历史商店全额退款桥的冻结实现通过 4 个定向 Go 用例，另有当前共享源码下 1 个 Core 客户端凭据与版本边界用例通过；不等于真实 Core/PostgreSQL 资金操作验收。完整来源与账本凭据导入、组合编译及退款资金恢复仍待验证。


- zconv 请求转换重构已提交 `7b3d684`，六向请求矩阵及相关旧用例通过并独立复核；保留原 Core 选中 Chat 路由的工具默认行为。完整响应和 SSE 转换仍使用原实现，本次没有宣称由 zconv 接管。
- 完整 Core 库组合通过 `cargo check --lib`，对应实现固定为 `5d89145`；可执行入口随后发现遗漏的数据库能力字段，修复为 `6c92d44` 后 Core 可执行文件实际构建成功。原生 `init-db` 实跑因旧 62/null 目标校验值返回 Storage、事务未提交；已按当前制品对应的真实 65 组件 SQL 捕获并独立重算目录指纹，提交 `68af4c1` 校准目标。含该校准的重建和原生复验尚未闭合，旧制品构建通过不代表最终候选通过。
- Go 扩展完整可执行文件构建成功，815 个冻结输入前后零漂移，实现已固定 `6fcf501`。现存本地构建包含提交前的 dirty VCS 标记，已保留精确源码与制品哈希，尚未进行正式发布资格验收或生产部署。
- 订阅原单迁移工具已提交 `ad7ce05`，独立 HEAD 加三个候选脚本的 11 项测试通过。SQL 层实际验证 18 笔完整材料和 19 笔 UNKNOWN 分流、并发单次权益、重放、错误额度回滚、冻结套餐不可变和最小权限；并发冲突保留序列化失败及原结果重放证据。未覆盖支付签名、Core/Go 网络、完整金融目标审批及生产迁移。
- 生产商店库存只读核验：50 条密文全部通过远端内存 AEAD 认证，其中可售 46 条；无密钥或商品明文导出。目标库存密钥、重封迁移、提货和资金恢复仍须验证。


- 校准 65 组件目标后，固定 Core 制品 `40fd783f…d3b02c` 的真实 `init-db`（含 schema check）、完整目录指纹、初始数据及 owner ACL 检查通过。基线运行自检暴露离线商店导入函数名称解析权限问题，修复 `93e1290` 改为精确签名 OID 检查；实际 SQL 验证无 schema USAGE 仍能检查、误授导入 EXECUTE 会拒绝、撤销后恢复。含该修复的新可执行文件尚待构建复验。
- 同一 Core 制品加冻结 Go 制品 `f769f809…29ae6` 的真实图片 HTTP 首例失败，返回 502 settlement_pending；原 trust SQL 对新 native-token tariff 仍按固定每图价格重算预留，在 COMMIT 拒绝。已定位，计费分支、折扣精确舍入及权限接线修复中；旧图片 SQL PASS 不能替代这项业务失败。
- 固定上述 Core/Go 制品的真实历史退款合成订单联测通过：两笔原单各一份退款回执、一次资金流水；Go ACK 丢失后服务 worker 只读恢复；余额不足 UNKNOWN 完整回滚、GET/worker 不执行、补足既有资金后原卖家请求显式继续；总账守恒、不创建 native pay、不返库已交付商品。根独立审阅实际用例和资金快照通过。首轮测试漏挂商店公开路由的 fixture 失败保留；这不证明生产四笔原单迁移、完整金融导入准入或正式发布通过。离线原单退款材料工具 `30cb48e` 在 HEAD 加两候选文件隔离环境的 5 项测试通过。


- 图片 trust 预留 SQL 修复已固定 `459ea67`，原固定单价路径及后半资金/回执约束保持。实际 SQL 验证 native/legacy token、精确折扣、预算/计数篡改与权限拒绝；两次 fixture 授权失败保留。新目录目标 `f7b4004` 独立重算后，新 Core 制品 `106b74e2…750782` 构建成功并实际通过原生 init/check、65 组件目录与基线 runtime check-db。
- 上述新制品的合成上游图片 HTTP 复验通过：GPT2 gen/edit 各结算343、预留1710；Gemini 比例模式结算28、预留160；flare 请求3返回2、结算900000/预留1350000；UNKNOWN 保留1710且原请求重放不再派发。钱包总额独立复核。未覆盖 OpenRouter Chat 图片、原生 Gemini wire、跨64未知队列公平恢复、崩溃重启或真实付费供应商。仓库复跑入口 `scripts/verify_image_process.py` 已固定 `3d03997`，仅适配预检通过，缓存实跑与仓库适配版本明确区分。
- fresh65 审核合同及金融工具衔接已固定 `2eaec96`，31 项源码与65组件指纹实核。完整离线金融导入44项、就绪19项重跑通过（176秒/62秒）；这是合成合同测试，不是实际金融导入准入。
- 新真实生产来源归档 SHA256 `3b252cd57dd6923b1d69eaa9a8eccf4c285c84aeb8c7e12679cb62f923d2330d` 已只读导出并在本机隔离恢复。213表/120序列，owner/reader逐表计数一致、reader权限反例和元数据分类检查通过；用户1246、Key878、OAuth绑定246、充值327、用户订阅15。原归档和秘密只存私有目录；审计脚本失败及续验记录保留。尚未执行真实身份或资金导入，未验证生产 writer fence，重启隔离源须刷新实例回执；未部署生产。

## 基线与使用方法

本表依据产品源码 [53c945dc][source]、旧稳定版 [go-v0.2.118][legacy] 和 [2026-11-08 固定审计][audit]。本次文档整理没有更改产品实现，也没有重新执行业务验收。后续实现提交须逐项补充自己的源码、产物与验证记录；不能把文档提交当作产品进展。

| 状态 | 含义 |
| --- | --- |
| 待补齐 | 已确认存在功能或行为差距，需实现并验证 |
| 待修复 | 已复现的工程合同或交付配置问题 |
| 待验收 | 已有实现、工具或历史实测，当前候选的完整证据未闭合 |
| 待前置 | 依赖实现、独立验收或交付输入先完成 |
| 已验收 | 本项范围由固定输入及独立证据证明；注明日期与验收者 |

每项以表中关闭条件为准。开始实施时记录负责人与候选提交；申请关闭时记录实际命令、结果、原始证据位置和独立验收者。失败、未运行和范围外行为分别说明。当前列出 **39 项推进事项**，大小与依赖不同，不用项目数换算完成率或工时。尚未验收不等于从未实现或从未测试。

## 首先解除的阻断

审计基线的边界检查为 32 项、26 项通过、6 项失败；完整质量门禁在计划阶段被过期且仍为 open/critical 的 QD-001 阻断，`checks=[]`。这些原始记录保留在[固定审计][audit]，本轮修复与独立复验见下表及[原始证据][rel-evidence]。

REL-01 的既有静态边界阻断已解除；REL-02 已补齐下表固定候选的真实镜像资源与启动验收。新增实现改变存储合同或运行输入时，仍须复验相应范围。CHECK-09 仍需逐条补齐 QD-001 退出证据；不得通过只改指纹、延后期限、降级债务或手填通过记录消除阻断。

2026-11-08 固定候选为 [7fb02902][rel-candidate]，基于 `bb2697e9216d67bf1f34705af570ed2d8eb5288d`；开发分别由 `rel01_boundaries`、`rel02_delivery` 负责，`rel_acceptance` 独立审查并执行检查，验证前后工作树干净。真实 PostgreSQL、容器及资金业务验收未运行。

## 工程与交付

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| REL-01 | 静态边界合同已对齐：模块连接池、4 处已审查正文指纹及 Core schema 拒绝范围 | 已验收 | 2026-11-08，`rel_acceptance` 在 [7fb02902][rel-candidate] 独立核对池归属/关闭、GRANT、namespace 与安装权限，33 项边界检查通过；补齐 directory_ads/referrals 及拒绝反例。[原始证据][rel-evidence]。范围为静态源码合同，真实数据库权限验收仍按 CHECK 项执行。 |
| REL-02 | 当前源码/main、TOML 与固定来源已对齐；固定候选真实运行镜像已验证 | 已验收 | 修复 [15bc5748][rel02-fix] 的既有静态证据保留。2026-11-10，`Maxwell` 从 `c9ed1de2deed81bc5569f0a29d18b9b57caa1ac6` 私有 git archive 实际构建 Extensions runtime linux/amd64，镜像 `sha256:47c775673f82ee8bc947d68183efd2a249a3cf9db999997af81d0a3a3c55fd21`；原版 smoke 验证资源/许可哈希、UID 65532、无网络/只读根、liveness 200 和正常退出 0，容器清理读回通过。证据：`~/.cache/lmm-rel02-c9ed1de-mhjivaya/REPORT.txt`。首次 mise 信任拦截日志保留，固定镜像只调 PATH 后复跑通过。未包含随后 APP/MODEL/ASSIST 增量；最终组合、发布和生产按 REL-03/CHECK 项继续验收，未推镜像或启用 Actions。 |
| REL-03 | 当前产品组合的制品、安装升级与发布证据未闭合 | 待前置 | CHECK-09 关闭后，关联 Core、Extensions、CLI、Web 的来源、平台、摘要/签名、安装升级记录与 Release；发布和生产切换分别记录。 |

## 身份、模型与助手

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| AUTH-01 | 通用 OIDC、WeChat 登录/绑定及旧 OIDC provider 合同未对齐 | 待补齐 | 补 provider、claims、issuer/client 校验、绑定与撤销和实际页面。保留已实现的 OAuth2 服务端、MFA 与 Passkey，分别验收。 |
| MODEL-01 | 视觉输入与 Anthropic cache_control 切片已实现；hosted tools/fileData 与组合验收仍未齐 | 待补齐 | 2026-11-09，`model_identity` 实现 [df9b3c9d][vision-input]，`root` 独立审阅：Chat/Responses 图像 URL 与内联数据、Anthropic URL/base64、Gemini inlineData；保持图文顺序/detail，拒绝有损转换，Core 不下载客户端图像 URL。`cargo check --lib --locked` 通过；此前三次 codegen 被 SIGKILL，新增单元、真实 TCP 与 PostgreSQL 计费用例均未执行，不据此声明协议/资金验收通过。[原始记录][progress-evidence]。2026-11-10，`86f6925` 保留 Anthropic 顶层、工具、system、消息块和工具结果的缓存标记/TTL/原边界，跨协议有损转换明确拒绝；`Poincare` 开发自测后，`Maxwell` 独立冻结 `86f6925`，精准 Anthropic 20 项、vision 5 项通过（25 个唯一测试）；旧宽筛选 33 次执行含重复/无关匹配，不计作 33 项覆盖。证据 `~/.cache/lmm-model01-independent-023fvl82/REPORT.md`。未验证真实提供商及计费组合，文档块/多块工具结果仍明确拒绝。继续补 hosted tools、带归属校验的 Gemini fileData/provider file_id，并运行四协议响应、usage、唯一结算与恢复测试。 |
| MODEL-02 | Responses 持久化/previous_response_id 与部分旧同步 API 缺等价行为 | 待补齐 | 补存储、续接、工具、重放与计费；逐项恢复所需 completions、compact、search、decisions、moderations 等实际行为。 |
| MODEL-03 | 独立 TTS/STT/translation 和双向 Realtime/WebSocket 未齐 | 待补齐 | 补连接、取消、回执、计费与断连恢复；当前 Chat audio 与模型 SSE 继续保留，不能据此关闭本项。 |
| MODEL-04 | 视频、Kling/Jimeng、MJ、Suno 异步任务未形成完整闭环 | 待补齐 | 按任务族补提交、查询/回调、结果/remix、原操作恢复与唯一扣费。 |
| MODEL-05 | 渠道 tag、多 key 轮换、Codex 运维和真实推理探测未齐 | 待补齐 | 补自动选择/失效/恢复与真实调用测试；现有渠道 CRUD、密钥库、价格发布和目录探测保留。 |
| ASSIST-01 | 预设、Core/Go 实时流及 Web 原请求恢复已实现；最终组合仍待验收 | 待补齐 | 2026-11-09，`model_identity` 实现 [a0804b60][assistant-presets]，`root` 在冻结输入上独立验证真实 PostgreSQL/TCP 的发布、版本冲突、匿名计数和预设点击一致性；三个 Go 定向用例、webcompat 包、实际 TanStack 父/子路由八项用例及 Extensions 构建通过。L5 可读、L6 可写；预设只提供用户开场内容，不增加工具或资金权限。[原始记录][progress-evidence]。2026-11-10，`16bd895` 接通 Go 真实逐帧 SSE、原 turn 只读重连与显式取消；`Mendel` 对 assistant/webcompat/coreclient 的定向 race 自测通过，包括真实 HTTP 首帧 flush/断开与 gRPC/TCP 协议，未使用实际 Rust Core/账本；原记录 `~/.cache/lmm-assist01-validation/report.txt`。`33ef79b` 接通 Core 原计费请求的真实上游 SSE 与绑定 owner/input/round 的 live registry；`Mendel` 在冻结私有源码上编译并自测 21 项通过，真实隔离 PostgreSQL/受控 HTTP 上游证明前缀提前可见、原 usage 唯一结算、取消/截断保留 hold 及独立实例恢复不重发。开发证据 `~/.cache/lmm-assist01-validation/core-report.md`。`Dirac` 核对固定来源与测试二进制哈希后，在新建私有 PG18.6 独立执行 21 个唯一用例（2 个 SSE、8 个 provenance、11 个 SQLx/PG）全部通过、零跳过；首次一项因验收 TMPDIR 太长失败，仅修短路径后复跑该项，失败保留。证据 `~/.cache/lmm-predeploy-validation/20261110-independent-assist-core-run/report.md`；这是独立运行开发产物，非独立编译，跨实例测试仍是同一 OS 进程内对象。实际 Watch/Cancel RPC→Go→Web、真实进程重启、真实外部提供商与最终组合未验收。`66e8e9b` 已提交 Web 首次 ready 前断线的只读定位、明确拒绝、用户显式 receipt 恢复与原 turn 取消；开发自测五个隔离 Bun 进程共 169 项通过，类型/lint/格式通过，证据 `~/.cache/lmm-api/assist-01-web-recovery/20261110T080029Z/report.md`。`0b1c3a3` 补齐 Go 同一创建键查询/receipt 恢复及持久化 pre-start fence；开发定向 race 回归与 3 项真实 PG/HTTP race 通过，覆盖只读权限、迟到 Chat 拒绝及先写入 intent 时保留 UNKNOWN，证据 `~/.cache/lmm-assist01-lookup/report.md`。后者的 Core 身份/登记端口仍为受控 fixture。随后 `Dirac` 独立运行固定 Core `7f6a…` 与 Go `ec3210a` 组件的实际进程链，通过 Watch→Go SSE、prefix 早于 done、断线原 turn GET 恢复、已知 usage 唯一扣费、取消保留待对账资金和会话/归属拒绝；[固定组件证据](../.quality/evidence/assist-osmain-20261110.md) 保留此前全部 fixture 失败。浏览器 cookie/CSRF、最终组合及不确定提供商后续对账仍未验收。Web 类型限定修复见 CHECK-06，不能代替最终组合。 |

## 订阅、支付与商店

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| SUB-01 | 外部支付订阅缺口：EPay 一次购买及 Stripe/Creem/Pancake 的原支持生命周期 | 待补齐 | 按 provider 补产品快照、首购/周期 grant、失败、取消和原交易恢复；钱包充值与余额订阅分别验收。2026-11-10，`d24577e` 提交 EPay 原生购买、冻结报价确认、原单只读 checkout 恢复和 root 价格管理页；开发逐文件自测 15 项、类型检查与隔离 Web 构建通过，新增提示七语言已补齐，证据 `~/.cache/lmm-api-sub01-web-epay/admin-v2/manifest.json`。该前端结果使用受控 HTTP/DOM fixture。`3579ac7` 已提交 Core59 EPay 原始凭据、唯一 grant、Go 持久化回调恢复及离线升级命令；开发真实 PG/TCP 四项用例通过，覆盖伪造回调不阻塞有效凭据、丢 ACK 后恢复、权限恢复后无需支付商重发、取消/改价/轮换密钥后的原交易和共享购买上限。证据 `~/.cache/lmm-sub01-validation/rust-epay-final.log`、`go-upgrade-final.log`；提交中的 810 个 Core 构建输入与 `core59-final-inputs.before.json` 一致，产物摘要 `7f6a0fdc9a5a8ca4c48fa7c05d1f639c09850af4274f0ad4b6c3d2a72d532c00`。`5ab3159` 补齐 root 管理价目 GET，并修复原支付渠道临时变为其他 provider 时把有效回调误判为永久拒绝的问题；一次限资源合并编译及两项真实 PG/TCP 定向自测通过，恢复 EPay 配置后无需支付商重发且仅一个 grant。详见 [合并验证记录](../.quality/evidence/sub-app02-core-20261110.md)。实际浏览器、固定组合独立验收和其他 provider 生命周期仍未完成。 |
| SUB-02 | 管理记录与立即失效已实现；reset voucher、绑定/删除与批量重置仍未齐 | 待补齐 | 2026-11-10，`384f531` 新增 Core 管理记录、原生用户抽屉与 root 失效操作；`root` 自测后由 `Dirac` 独立在 PostgreSQL 18.6 复跑实际查询、过滤/分页、导入额度、8 次并发原操作恢复、唯一回执/审计与审计失败回滚，资金/消费/预留保全；页面四项定向自测通过。证据：`~/.cache/lmm-predeploy-validation/20261110-independent/report.md`。仅 SQL 与组件证据，Rust HTTP/会话权限/charging 进程及浏览器未验收；继续补 voucher、绑定/删除、批量重置及完整授权/额度语义。 |
| PAY-01 | 商店仅接 balance/free，缺四类旧外部收款 | 待补齐 | 恢复 external:epay、external:waffo_pancake、platform:waffo_pancake、platform:linuxdo；分别证明卖家/平台收款、币种、库存交付、结算和恢复，保留 LDC 单位。 |
| PAY-02 | Pancake 及外部商店退款未齐 | 待补齐 | 补原交易冻结、发送、查询/通知、失败释放、对账及卖家收入回冲；已有 Stripe/Waffo Global 退款与 Core ReleaseRefund 按现合同验收。 |
| PAY-03 | Pancake catalog/pair/save、订阅映射与 customer portal 未接完整后台 | 待补齐 | 接当前配置和权限、秘密只写、server 绑定 customer/store/environment，并完成真实管理页面联动。 |
| STORE-01 | 游客购买与邮件提货合同未齐 | 待补齐 | 补游客订单归属、邮箱验证、付款领取、邮件 outbox/重试；覆盖凭证混用、重放及退款后禁止领取。登录买家的提货码/链接已有。 |
| STORE-02 | 商店按数量/金额的部分退款缺失 | 待补齐 | Core 与 Go 绑定原订单和每次退款身份；证明累计上限、库存/交付、卖家收入、外部回执及丢 ACK 恢复。 |
| STORE-03 | 三态目录、管理页与显式卖家自购已实现；实际 Core RPC 和浏览器验收待补 | 待验收 | 2026-11-09，`commerce` 实现 [63db64bc](https://github.com/TokenNotIncluded/lmm-api-source/commit/63db64bc0124f0361f0f5a7facde72b0c904d46a)、[6d74cefd][core58] 与 [9b377046][store-self-purchase]。`root` 独立复验真实 PostgreSQL 的可见性/分页、56→58 原数据与权限保全、自购全额预留/结算/退款和八组并发原操作重放；订单/提货/退款用例使用合成资金回执，实际 Core 账本 SQL 另行验证，两者不冒充 RPC 闭环。页面八项可见性测试通过，checkout 全文件 31 项通过并发现一个旧库存 fixture 缺少能力参数，在旧基线复现后补足该输入，原断言定向复验通过。能力由 `commerce.self-purchase.v1` 明确协商，`store_access_supported` 继续关闭。[原始记录][progress-evidence]。新 Core 组合编译被 SIGKILL，实际进程资金链、全量 Web 类型检查和浏览器操作仍待验收。 |
| STORE-04 | 账户购物车、收藏、清理和游客合并已接通；完整页面验收待补 | 待验收 | 2026-11-09，`commerce` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 PostgreSQL 的账户隔离、绝对数量并发重放、重启读回、私有化隐藏、暂停保留及下架/变体清理；既有库离线迁移与精确 DML 权限用例通过。已有页面按 `max(server,guest)` 合并后发送绝对数量 PUT；能力已开放，不预占资金或库存。冻结 Extensions 构建通过。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL，浏览器操作与最终组合按 CHECK-06 继续验收。 |
| REVIEW-01 | 市场 AI 自动审核缺配置、执行和结果历史 | 待补齐 | 恢复 off/assist/auto 与发布/拒绝边界，接审核设置和权限；保留现有人工审核与版本发布。 |

## 业务后台与页面

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| APP-01 | 公开与私有 GitHub 配置、原子拉取及快照已实现；实际 Core/浏览器组合待验收 | 待补齐 | 2026-11-10，`2e2ae58` 实现配置 revision CAS、受限 archive 读取、完整快照与来源持久化、operations v3 与离线升级、管理页保存版本和草稿禁止 pull；`Dirac` 独立复验 39 项边界合同、operations race 15 项零跳过、真实 TLS/PG 并发与失败保留、v1→v2→v3 原操作恢复、最小权限及 composition/离线 CLI，证据 `~/.cache/lmm-predeploy-validation/20261110-independent-app01/report.md`。Web 定向用例和完整类型检查自测通过；Core 授权为合成 fixture，当时私有仓库密钥明确 501。后续 `9c9ef8a` 补加密凭据写入/替换/清除、L6 新鲜授权、受限私仓 archive 请求、显式离线凭据扩展和独立 32 字节 keyfile。开发实际 TLS/PG/race 与 Web 定向测试通过；首次全站类型检查遇到同时编辑的 SUB 测试类型错误，后者已由 SUB 作者修复并通过类型检查。证据 `~/.cache/lmm-app01-private/report.md`。root 另完成真实 PG 离线 CLI/原收据恢复/不同操作拒绝和密钥隔离自测，证据 `~/.cache/lmm-script-cli-30zvxt6z/report.md`。私仓网络为受控 TLS fixture，无真实 GitHub PAT；独立验收、实际 Core 与浏览器全链路仍待完成。 |
| APP-02 | 用量分享 Go 实时来源、原生 SVG 与 Core 全历史查询已实现；完整验收待完成 | 待补齐 | `beae53f` 接通原生 profile/models/用量卡片、按用户读取 LMM 数据和 Cursor 公开数据源，保留显式 snapshot 来源；金额未知不当作零，撤销或权限变化使在途结果失效。开发通过实际 Service.Register 路径的 9 项定向用例（含 6 个参数子场景），覆盖布局、公开字段、混合未知价格、所有者隔离、撤销及 Cursor 网络/缓存规则；11 文件运行前后哈希一致，证据 `~/.cache/lmm-app02-validation/inputs.json` 与 `test-final.log`。以上 Go 测试使用受控 Core/数据库/网络 fixture。`5ab3159` 的 Core 全历史查询、nullable 金额和显示汇率已通过一次合并编译及三项定向自测，其中真实 PG/Rust 路由覆盖用户隔离、退款、历史来源优先级及混合未知价格，详见 [合并验证记录](../.quality/evidence/sub-app02-core-20261110.md)。`6e2f432` 补充汇总全部历史选择，页面单文件 7 项自测通过。Core 与 Cursor 已改为并行读取并各用自身超时预算，原请求取消仍同时传播；两项门闩定向自测通过，证据 `~/.cache/lmm-app02-validation/timeout-coupling-fix/manifest.json`。新 Core `b64ad8…` 与固定提交 Go `a02dbb…` 的实际服务二进制均已构建成功，输入前后指纹一致，见[组件构建记录](../.quality/evidence/app02-component-builds-20261110.md)；固定 Core/Go 实际进程组合、浏览器与 Cursor 公网仍未验收。 |
| APP-03 | HeroSMS 当前目录、报价、购买、取消、重下单和管理缺失 | 待补齐 | 补真实供应商 IO、Core 资金、退款和页面接线；现有历史只读不变。 |
| APP-04 | io.net 远端模型部署管理 | 待验收 | 已实现设置、创建/列表/容器日志、续期/更新/改名/删除、L5/L6 权限及原操作回执/UNKNOWN/取消围栏。6 项 Go、5 项 Web、类型检查和 5 项实际 PG 开发自测通过；已验证提交基线加本项 35 文件的四入口编译，保留首次路由失败及并行来源漂移边界。[开发证据](../.quality/evidence/app04-ionet-development-20261110.md)。随后独立实际 CLI 在 `a32f98c` 上发现 runtime 数据库 CREATE 权限未被升级拒绝，原验收 FAIL；随后独立固定 791 输入实际 CLI 复验通过：CREATE/TEMP 的直接及 PUBLIC 授权在安装/恢复时均拒绝，撤权后原操作正常升级恢复；原失败保留，仅关闭此权限缺陷。Core 进程授权、浏览器视觉/交互、真实 io.net 合同及最终独立验收仍未完成；与本项目安装发布工具分开验收。 |
| APP-05 | 平台充值折扣码缺管理、校验与新消费合同 | 待补齐 | 补范围、用户/金额核验、次数/并发、消费绑定及旧数据迁移。商店优惠券与充值兑换码已有，属于独立行为。 |
| APP-06 | 公开安全报告与 Moderation 后台未完成旧行为映射 | 待补齐 | 对照 native abuse，逐项补 policy/stats、审核队列/任务/统计和页面操作；已有策略、标记、审计继续保留。 |
| APP-07 | RSS 抓取、缓存、设置保存与页面接线已实现；线上 edge 与浏览器验收待补 | 待验收 | 2026-11-09，`business_apps` 实现 [c45beffa][collections-rss]，`root` 独立复验真实 HTTP 抓取、RSS/Atom/RDF 投影、并发/缓存/错误与网络边界；三项真实 PostgreSQL 用例证明配置持久化、并发版本校验、v1→v2 离线升级及原操作恢复、HTTP 权限与禁用源不抓取。七项定向 Go race 测试、冻结 Extensions 构建及 34 项边界检查通过。配置仅授予 SELECT/UPDATE，普通启动不执行 DDL。[原始记录][progress-evidence]。全量 Web 类型检查被 SIGKILL；实际 Nginx 路由与浏览器验收仍需目标环境。 |
| APP-08 | 价格变更 webhook 投递与重试后台未迁移 | 待补齐 | 补订阅接收人、outbox、签名 HTTPS、持久状态与幂等重试；现有站内通知另行验收。 |
| APP-09 | Core 报价读取、展示汇率保存及状态回读已补齐；实际进程与公网验收待补 | 待验收 | 2026-11-09，`business_apps` 实现 [6d74cefd][core58]，接已有 `33ba2e1` Web；`root` 独立验证真实受限 PostgreSQL 的专用 setter、输入边界、无直接表写/DDL、失败事务回滚及原始余额/账本逐行不变。现有页面七项定向用例通过，明确区分报价日期与下载时间。Core56→58 的实际 SQL 保留/权限验证通过；新组合 `cargo check --tests` 与 `--lib` 均被 SIGKILL，Rust HTTP/报价网络 fixture 未运行，Frankfurter 公网及浏览器验收未完成。[原始记录][progress-evidence]、[升级合同](commerce-display-rate-upgrade.md)。 |

## 客户端

| 编号 | 目标与当前差距 | 状态 | 下一步及关闭条件 |
| --- | --- | --- | --- |
| CLI-01 | 锁定 CLI 仍为开发预览，多软件与多平台接入未齐 | 待补齐 | 按目标软件/版本完成真实接入、可恢复写入、凭据与失败路径、安装/同步/更新/解除/卸载/运行和签名分发；预设目录数量不代表已接入数量。 |

## 迁移与独立验收

这些项目保留已有实现及历史实测的价值；关闭时需证明当前最终组合，不从旧报告自动继承通过。CHECK-04、CHECK-05、CHECK-06 共用冻结候选和证据标识，分别证明真实宿主、协议/权限与用户操作。

| 编号 | 验收目标 | 状态 | 关闭条件与最小证据 |
| --- | --- | --- | --- |
| CHECK-01 | 身份与权限迁移 | 待验收 | 2026-11-10，`Maxwell` 冻结 `1e0937b` 核验：源码声明 58 组件，安全 importer 的 reviewed profile 仅到 56；实际调用拒绝 `component_rust_source_fingerprint_mismatch`。现存核验过的缓存 Core 分别为 54/55，不能用于当前候选。证据 `~/.cache/lmm-check0102-facts-cbpzd8wo/REPORT.md`；后续 `ccc21e9` 已审查 56→58 DDL/ACL 并补齐安全 profile 与升级收据权限检查；开发单测 167 项通过、1 跳过，真实 PG18 合成 dump→restore→身份/资金导入及两阶段重放通过，数据与序列保持一致，越权 ACL/已有升级收据实际拒绝。证据 `~/.cache/lmm-safe-importer-dkqkkslg/REPORT.md`；这是 58 profile 开发证据。随后安全 importer 已接入固定 `3579ac7` 的正式 59 profile，绑定已审源码、编译 manifest、实际 catalog、ACL 与空目标围栏；59 定向单测 62 项及历史 profile 回归 5 项通过，证据 `~/.cache/lmm-importer59-reviewed-p7mlo7oo/REPORT.md`。随后开发在初始 Core59 二进制 `07930fa8…` 上运行隔离 PG18.6 合成源 dump→restore、身份/资金 SQL 导入与即时重放，通过债务 -120、推广余额 -5、钱包 43 及高权限用户休眠回读；证据 `~/.cache/lmm-importer59-actual-run-zpr9llcm/REPORT.md`。该次源仅为三用户 payload fixture。随后 `e77d463` 修复 exporter 两个 reader 会话的时区误判，4 项定向回归及真实 PG 的物理合成源→身份→资金→订阅→原路由→凭据连续迁移开发自测通过，含目标回读、原钥匙恢复、重放与跨阶段拒绝；[连续链证据](../.quality/evidence/core59-continuity-chain-20261110.md) 保留首次失败。仍为固定旧 Core59 组件的合成来源，实际旧站来源、最终候选独立验收与新 schema profile 未完成；后续修改须同步精确来源指纹。未改用宽导入入口绕过。需匹配最终 Core 的实际安装与 catalog receipt 后运行真实迁移。仍须同一来源/目标的身份、特权、团队/开发者、密码/会话/Key/OAuth/MFA/Passkey 映射；实际登录/撤销、deny 优先、跨租户和并发批准，旧凭证处置与全状态回读。 |
| CHECK-02 | 资金与持续权益迁移 | 待验收 | 钱包、欠款、返佣、订阅、Key 限额、预留与持续权益具备来源和单位；逐笔/逐账户对账、账本平衡，原操作恢复不重复扣款。`e77d463` 的[合成连续链开发自测](../.quality/evidence/core59-continuity-chain-20261110.md) 已覆盖导入资金、订阅当前/未来周期、有限/无限 Key 额度及阶段重放；不能代替实际旧站来源、运行中计费与独立迁移验收。 |
| CHECK-03 | 历史读取与导出 | 待验收 | 实际日志/用量/统计/财务的来源目标范围、时区、分页、筛选、unknown 金额、权限和导出字段逐项对照。 |
| CHECK-04 | 支付、商店、客服及商业资金闭环 | 待验收 | 当前 Core + 实际 Go main + PostgreSQL；重复/乱序、撤权、并发消费退款、丢响应、双进程重启及配置变化后的旧订单。合成 provider 与真实支付商分别报告。 |
| CHECK-05 | 模型、助手、MCP、工具与 HumanGrant | 待验收 | 所需协议的 JSON/SSE/usage/取消/重试/冻结价格；真实对话、赞助付款方、跨租户/撤权/额度反例、恶意工具输出与原操作恢复。 |
| CHECK-06 | 全站页面、CLI 与跨仓库集成 | 待验收 | 2026-11-10，`b2e97f9` 修复 Extore 目录数组交叉类型与助手测试事件类型；`Dirac` 独立执行完整 Web 类型检查及逐文件组件测试（Extore 1 项、助手设置 8 项）通过，固定输入与限制见 `~/.cache/lmm-predeploy-validation/20261110-independent/report.md`。仍需按角色/语言/桌面与窄屏/错误状态执行，截图、请求与数据库回读绑定最终候选；实际管理弹窗、OAuth consent、广告与 Extore 两端授权/多规格/重复同步/撤销/隔离。Extore main 自动部署限制继续生效。 |
| CHECK-07 | 安装、升级、备份恢复与切换 | 待验收 | 当前 schema 的 owner/runtime 分离、普通启动无 DDL、低权限拒绝；实际隔离恢复、writer fence/drain、死 holder CAS、有效写入保全与业务重启。 |
| CHECK-08 | 资源、旧新版性能与稳定性 | 待验收 | 相同 CPU/内存、数据、功能和计费语义下的优化构建；完整成功率、吞吐、P95/P99、RSS/PSS、长稳、慢消费者和故障恢复；产品 cgroup 与整机预算分开。门槛见[质量指南](quality.md#独立业务与资源验收)。 |
| CHECK-09 | 最终冻结组合与 QD-001 关闭 | 待前置 | 前置实现、迁移和验收全部关联同一源码/产物/配置/来源/目标；不同人员/agent 独立复验，逐条满足 QD-001 的真实 host/funds 条件，保留失败记录并确认无输入漂移。2026-11-10，`Peirce` 在固定初始 Core59 二进制与 `ec3210a` Go 主机产物上独立执行同一订单双 HTTP settle；两请求实际锁等待重叠，均返回相同成功回执，唯一 capture/settle operation 与余额、账本平衡回读通过。[组件证据](../.quality/evidence/qd001-same-order-settlement-20261110.md) 保留前两次 fixture/清理失败及来源漂移范围；随后固定 Core `7f6a…` / Go `b68c…` 实际进程组合通过权限拒绝、唯一结算、原操作重放及真实退款已提交但 Go 回执持久化失败后的只读恢复；[资金恢复增量证据](../.quality/evidence/qd001-funds-recovery-20261110.md) 保留首次源码漂移拒绝与冻结快照修正。组件不含后续 SUB/APP 改动，最终冻结组合仍须复验，QD-001 保持 open/critical。 |

## 维护约定

- 产品范围和支付支持边界分别以[完整验收合同][acceptance]、[支付矩阵][payment-parity]为准；删掉旧任务文件不缩减需求。
- 专题指南解释接口、配置和操作，禁止复制本表的动态状态；变更进度只更新本文件。
- 关闭一项时，在该行附固定提交/产物、证据和日期；独立验收未完成时保持“待验收”。产品缺口解决不等于已发布、已上线或完成真实迁移。
- 旧审计、失败记录与任务提示词从[历史索引](archive.md)访问。历史内容只能证明其冻结输入和实际执行范围。
- 当前文档更新时间不代表线上版本；公开版本规则与私有切换身份见[发布说明](release-architecture.md)。

[source]: https://github.com/TokenNotIncluded/lmm-api-source/tree/53c945dccb3ef1a105261bc6a91c01dfc6f26191
[legacy]: https://github.com/TokenNotIncluded/lmm-api/releases/tag/go-v0.2.118
[audit]: https://github.com/TokenNotIncluded/lmm-api-source/blob/0eae39f10ce8a1d80744ad44e88807e01facfda5/docs/architecture-parity-audit-2026-11-08.md
[acceptance]: https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PRODUCTION_ACCEPTANCE.md
[payment-parity]: https://github.com/TokenNotIncluded/lmm-api/blob/main/recipes/lmm-api/PAYMENT_PARITY.md
[rel-candidate]: https://github.com/TokenNotIncluded/lmm-api-source/tree/7fb02902eefd21670840125c73bea539fab853ce
[rel02-fix]: https://github.com/TokenNotIncluded/lmm-api-source/commit/15bc57486442a3a2deae538ba7105069c6dd9c81
[rel-evidence]: https://drive.google.com/drive/folders/1c1MpzgGZ7yDk7F53b28aDeoOfx8h9zxy
[progress-evidence]: https://drive.google.com/drive/folders/1WpvJDD0IDjB8p-qr14k9_lgP9aWZ2hMw
[collections-rss]: https://github.com/TokenNotIncluded/lmm-api-source/commit/c45beffa4bb0aa4880f465788a6955708ae13ab6
[vision-input]: https://github.com/TokenNotIncluded/lmm-api-source/commit/df9b3c9d7c767248f9060905e24dd5c4212b065e
[core58]: https://github.com/TokenNotIncluded/lmm-api-source/commit/6d74cefd830e2805cfc4ac42e9558ca95a8cde9f
[assistant-presets]: https://github.com/TokenNotIncluded/lmm-api-source/commit/a0804b601375914109ca1598f5d26e82c38af38e
[store-self-purchase]: https://github.com/TokenNotIncluded/lmm-api-source/commit/9b377046b82b89367c21474505e8bfb577a1bdb8

# 新架构进度

更新于 2026-11-20。此文件维护当前状态、剩余阻塞和证据入口；先前失败、历史候选和各轮实际回执继续保留。

## 当前状态（截至 2026-11-21 01:55 UTC）

**新候选 `6aeb0198` 的原完整 CI 已由六个 job 全部通过；原产品 293、952 驱动与控制 `8164659d` 的五组容量、真实 302 秒支付专项和 16 项浏览器验收也已完成独立核验。** 这些结果补齐了各自源码范围内的技术证据，952 基础 CI、a443 双架构 Linux 安装、Windows 真实安装／重复安装／卸载及六个客户端插件发行的既有结果继续保留。最终 main 尚未取得自身部署资格：main/cf 与 feature/8e 的运行合同仍须分别处理，真实主机启用及三项财务材料尚未完成，安装器原 50,000 B 体积门槛仍未达到。历史失败和每轮原始收据保留，新的成功不会改写旧运行结论。

| 工作项 | 精确来源与实际运行 | 当前结论 |
| --- | --- | --- |
| 基础版本完整 CI | `952d21c5`；[44802025200](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44802025200) | 六个 job 全部 SUCCESS |
| Linux 安装与实物 | `a4437981`；[44816699551](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44816699551) | 双架构 60 包、20 项原安装案例全部 PASS |
| Windows 真实安装回归 | 产品 `3bdace68`、控制 `537c80cb`；[44823854144](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44823854144) | PS7／PS5.1 各 90+40 项、五 CLI、repeat／uninstall／数据保留 PASS |
| 最新产品候选 | [6aeb0198](https://github.com/TokenNotIncluded/lmm-api-source/commit/6aeb01987fc14bc9c51fd7b3803b7b190ddbdd7c)；[44839310512](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44839310512) | 六个原 job 全部 SUCCESS；终态 raw、源码身份与原门槛已独立核验 |
| 最终容量／支付／浏览器 | 原产品 293、实际 952 commerce 驱动、控制 [8164659d](https://github.com/TokenNotIncluded/lmm-api-source/commit/8164659d4f0d86fd8be9de5b648f1620f37d68a0)；[44839545043](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44839545043) | 七项原验收通过；来源与清场已独审，full_payment_acceptance=false |
| main 整合 | 精确 main304 与 6aeb 三方预检 | 11 处文本冲突、Extore cf／8e 合同不同，尚未整合 |
| 来源校验修复 | [cebc346c](https://github.com/TokenNotIncluded/lmm-api-source/commit/cebc346c868e7d0c03bd82dab8d8f274ef7ec365)，独立 review 分支 | 原 4 项保留，完整 13 项纯元数据测试通过；main 与正式发布条件不变 |
| main/cf 配套修复 | [7df35ca4](https://github.com/TokenNotIncluded/lmm-api-source/commit/7df35ca4185ccc67de70e3f68a7db7a01ca154c6)，基于 cebc 的独立 review 分支 | RPM OpenSSL 依赖与支付清单已修复；12 项纯测试 PASS，完整字节及目录树已回读 |
| 安装器体积 | 产品 3bdace，原 23 路径／11 类 | 72,912 B，原严格 <50,000 B 仍 FAIL，需净减 22,913 B |
| 生产财务与切流 | 原授权材料复核至 2026-11-21 00:54 UTC | 三项真实材料仍缺；live target 为 NOT_CHECKED_OFFLINE，生产连接／查询／写入均 0 |

### 一、已通过的基础验收

[952d21c5](https://github.com/TokenNotIncluded/lmm-api-source/commit/952d21c5d5d849507478d8443f603c47e0126880) 修正 Web 测试四处旧版本预期，原 7 项／106 个断言保留。原完整 CI 六个 job 全部成功，包括 Core、Web、翻译、Market、Extore native 与汇总。Core 的 Rust／真实数据库、支付 live、Go contract／race、Store／support PostgreSQL、durable event recovery、protobuf／RPC 检查已完成；该正常 CI 仍不代替单独的真实 302 秒 expiry 专项。Core 原 raw 为 5,715,854 B，SHA256 `fcd8825cc6e5937484f4bbc85ffce8f219855203276d6e40507dcaa05daef605`；独立终态汇总 SHA256 `1c403662dea6e11f484f1a88f7b3bdd1643355e33dc1c4f9607374fcb96a7f3f`。六份 raw 各一次取得。旧 293 的 Core 成功与整轮旧 Web 失败继续分别保留。

[a4437981](https://github.com/TokenNotIncluded/lmm-api-source/commit/a4437981e6566c0fb12968a3b44204716d97676f) 已包含 Arch 缺失 `cmp` 工具的依赖修复，并按实际 ELF 为 RPM 声明 OpenSSL 库及各自符号版本，解决 openSUSE 不能使用 Fedora `openssl-libs` 包名的问题。glibc、Debian／Arch 的原版本下限保留。

| Linux 原门禁 | amd64 | arm64 |
| --- | --- | --- |
| source/package tests | 26 PASS | 26 PASS |
| 实际包 | 30 PASS | 30 PASS |
| 默认安装案例 | 7/7 PASS | 5/5 PASS |
| 可选 Extore GNU 安装案例 | 5/5 PASS | 3/3 PASS |
| 实际 DEB host／SO 的 native-flow | 6.93 秒 PASS | 6.98 秒 PASS |
| 原 job 耗时 | 24 分 37 秒 | 31 分 22 秒 |

两个 job 均保持原 40 分钟上限。23 个 LMM 源文件绑定 a443，71 个 Extore 源文件保留 8e；实际 DEB 字节与 GHA 加载的 pair 相同，4 个实际 RPM 的只读头查询确认依赖要求。两份 raw／两份 artifact 各首次取得一次。验收 index SHA256 `8e21aa4567fd2b90da3c879ae3b83bedbe35a2e9f44275b253c3ea5c0d94fc3a`，summary SHA256 `5bc63287dafb53e04bc9ed5974dfd3b5e0493fecd5b6cba4427b88cb45089b95`。容器安装成功不代替真实主机 data_dir、UID／权限、服务启用和迁移。

### 二、产品整合候选已推送，部署仍按最终源码准入

前一候选 [1660de294dd3db814d9aab2d183162b0bba54c8f](https://github.com/TokenNotIncluded/lmm-api-source/commit/1660de294dd3db814d9aab2d183162b0bba54c8f) 的 parent 为 a443，tree 为 `1c6f8b6150a8b6da084ff98fe4c3804d59506fa9`，独立分支为 `review/8e-product-candidate-1660de29-20261120`。293→952→749→a443 已证为同一 Extore 8e 线，前四处差异分别是 Web 测试、Arch smoke 依赖、RPM 打包器与测试。

1660 只在 a443 上追加 `tests/payments/verify_process.py` 和既有单测：精确 full 3／ignored 1／selected 2 filtered，原业务断言、180／480 秒限额和至少 302 秒真实等待保持。其余 5,579 个叶节点和全部 20 gitlink 不变。原 4 项 CaseSelection 纯元数据测试已通过；已存在的错误清单／重复项／失败摘要拒绝继续保留，并补充 missing hex、错误 ignored 和旧 filtered 拒绝。

独立验收控制器、browser／payment 投影、host-preflight 及其 workflow 不并入产品。来源整合审查 SHA256 `aab548462090514a55dc6e0f64bb568a14ed89841034257eb83e07c384af4474`，两文件候选 manifest SHA256 `b3b2368d5defc55207cba16b79e4e0ae76915f59fe7e9333c2d2bfe551722fe1`，GitHub commit／ref 完整读回收据 SHA256 `7314e39deeb8266a388b681f168ae1d30f5cbd298f9f9597d9ff2843ecd5bbe0`。

1660 的原 [完整 CI 44834510267](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44834510267) 于 23:08:11 UTC 创建。精确 head／ref、workflow 399255954、workflow_dispatch 与 attempt 1 已核对；单次 POST 返回 HTTP 204，使用原默认完整 CI 输入。在 2026-11-20 23:25:06 UTC 的原观察记录中，Web、Extore native、翻译、Store/tool-market 四 job 已 SUCCESS；Core job `136627400819` 的 `Workflow and source boundaries` 步骤已 FAIL，部分后续检查 SKIPPED，当时实际 Rust／数据库步骤仍在继续；该时点没有此候选完整 PASS。定位期间保留原运行和观察器，未提前取得进行中 job 的 raw。派发收据 1,103 B，SHA256 `6e2138e6447c1c8f3a99813a8de41f82052d92f068c225e4c5237783b542410b`。未改工作流、任务集合或预算。

旧 1660 终态与原始日志首错：

原运行已终态 FAILURE。2026-11-21 00:52:51 UTC 的原观察 seq95 记录四个组件 job SUCCESS，Core job 136627400819 于 00:51:52 UTC 以 FAILURE 结束，Implemented component checks 于 00:51:59 UTC 随之 FAILURE。Core 原 step11 是唯一失败步骤；step12／14／15／16 以及后置 setup-go 为 SKIPPED，step21–29（Rust／真实数据库、支付 live、Go contract／race、Store／support PostgreSQL、事件恢复、原 protobuf／RPC、证据和服务边界）均实际 SUCCESS。

Core 原 raw 5,704,266 B、25,782 行，SHA256 be0d327bf2d1dfca1f47ba4f8129c0685522b9ec63a361eaff54093fc7385076，只取得一次。2026-11-20 23:09:59 UTC 的原 921–938 行确认首错：test_native_conversion_installs_private_pair_with_measured_dependencies 经 test-distribution.py:303 调用原 packaging/linux/build.py:91，报 ValueError: cannot associate OpenSSL versions with needed libraries；该组 19 tests、1 error。此原云端错误与随后 6aeb 修正的 File／Name 关联夹具吻合；旧失败保留，不因新修复而更改结果。原终态 jobs 元数据 SHA256 0a82e5c6cf53d015b440d009d7ba4e4d0d7da69f2929ec1ff9df399113152288；Core 单次取得收据 SHA256 cef14bc629ce5b385f638406d74c9242f28b9e5c47acbf16e1c9a37a1bf69f14。

旧源码边界失败与后续步骤结果分别记录；后续步骤通过不能抹掉此前失败或改变整轮原结论。

旧 1660 完整终态独审保持 FAILURE，原收据 14,145 B，SHA256 `0978384bba0aeb8eaeac3fbc1319c4499f5cd3bd9fd2ee50fca87bdb0d01132a`；该收据不因后继通过而修改。

在以上整合候选上，[6aeb01987fc14bc9c51fd7b3803b7b190ddbdd7c](https://github.com/TokenNotIncluded/lmm-api-source/commit/6aeb01987fc14bc9c51fd7b3803b7b190ddbdd7c) 只修正 `scripts/test-distribution.py` 的既有 readelf 夹具。RPM 解析器需要把 OpenSSL 符号版本对应到实际依赖库；旧夹具仅给出平坦版本字符串，新夹具补齐真实 `Version needs` 中的 `File`／`Name` 关联，并核对原合并命令参数和 `LC_ALL=C`。解析器、所有外层原断言与负例均保持。原完整 19 项纯分发测试在精确 1660 上为 18 PASS／1 ERROR，修正后同一测试集为 19 PASS／0 SKIP；未执行外部子进程或产品。独立审查 SHA256 `b2f23ee5f3c6ad916552aa51dfedff510a55e104bb7f0f78f106162d2eaa9222`。

6aeb 的 tree 为 `ad53bf2b881926b6bd34231441af0af952ec6edb`，parent 为 1660，其余 5,580 个叶节点及全部 20 gitlink 不变。原 [完整 CI 44839310512](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44839310512) 在 2026-11-20 23:47:13 UTC 创建，精确 head／ref、原 workflow 399255954、workflow_dispatch、attempt 1 均已核对。单次 POST 收据 SHA256 `9cca65c426fedc88788603a0040ed62378d01b6fe55d1ebb3766f7737ff57bce`。新 Core job `136643465484` 的原源码边界检查已实际 SUCCESS，工作流、Go bindings、公开法律读取、Compose／RPC fixture 与 Rust 格式检查也已成功。v33 已冻结的 2026-11-21 00:10:23 UTC 原观察器记录显示，Web、Extore native、翻译、Store/tool-market 四个 job 均 SUCCESS，Core 正在执行原 Rust／真实数据库步骤，完整 CI 及最终汇总尚未完成。已终态的四份原 raw 各首次取得一次并独立核验：Extore 实际 host 两项非跳过通过，翻译 9/9，Store 原三项 PostgreSQL parent 通过；Web 的 765 份逐文件摘要合计报告 5,393 PASS／0 FAIL，原 Rsbuild 在 7.62 秒完成，不能把这个汇总冒称为去重测试数。三短 job 独审 SHA256 `ccb1df4b5cbfbf182d99fd840fc5096afa638c2f2c7b979b72411bed861fff7e`，Web 独审 SHA256 `2c815f14844f2dddd2a61e63cdcc10be74dfa208b09d40c47e38e054faa3ed27`。 旧 1660 的源码边界失败和原运行记录继续独立保留；本地旧夹具复现与该轮原 raw 的首错取证分别记录。

6aeb 本轮原完整 CI 已实际终态 SUCCESS。2026-11-21 01:30:32 UTC 的原观察 seq92 绑定同一 run `44839310512`／head／attempt；Extore native、Web、翻译、Core、Store/tool-market PostgreSQL 与汇总六个 job 全部成功。Core job `136643465484` 于 01:30:00 UTC 完成，Implemented component checks job `136680542162` 于 01:30:05 UTC 完成；Core 全部原步骤均为 SUCCESS，没有 job 步骤跳过。六份原 raw 各由原观察器首次取得一次，观察器于 01:30:36 UTC 自然结束，原 supervisor／watcher 两个 PID 均已退出，未重启、重派或延长预算。

实际首个主仓库 checkout 精确为 `6aeb01987fc14bc9c51fd7b3803b7b190ddbdd7c`。原 19 项分发测试现为 OK，关闭了旧 1660 的 readelf 夹具首错；Rust lib 为 1,837 PASS／0 FAIL／34 ignored，全部 161 条 Rust 结果摘要的失败数均为 0。原 assistant_model 13 项、公共 provider OAuth 6 项、completion_upgrade 2 项、navigation_options_upgrade 2 项均实际通过且各为 0 ignored。Go contract／race、Store／support PostgreSQL、原支付 live、durable event recovery 和原 protobuf／RPC 步骤也已成功。Go 原 JSON 中仍有 91 个 skip 事件，Rust 原 ignored 用例继续保留，因此这里不声称所有声明用例都执行或全仓库零跳过；Extore 和三项 PostgreSQL parent 的非跳过门槛由其独立 job 证明。

原 RPC 日志实际记录了全新安装／重复初始化拒绝、native teams／protobuf／独立凭据，扩展重启／替换／错误 service token，以及重连／有界数据库失败／持久密钥／立即撤销／关闭模型路由三组检查，随后报告 owned Docker 安装断言与清场通过。durable events 的原范围仍是 fixture events，不扩大为账本验收。Core 原 raw 为 5,717,351 B，SHA256 `23a9faedcccf37bdb6ff51d2d8be1586bd27f29d137bc64c34c86cb69717bff1`；关键摘录 3,968 B，SHA256 `d9001129d6ab89fbf1ae47a0e4d196832f193490b0b5f5de7bb37aabe46fa0f7`。完整终态独审 20,143 B，SHA256 `062459ca57f4f33cf1b97fdb2e85afd3a709f11dd94a5606c5fda6d7ae62ebdc`。本结论是该精确 review 候选的原完整 CI 资格；汇总 job 本身仍明确不授权生产部署。

只读整合预检确认，当前已核对的 main 与 6aeb 不能直接快进合并。[main304](https://github.com/TokenNotIncluded/lmm-api-source/commit/304f49d87df1c2fb83d4145386c569d237af8c90) 的 tree 为 `c1913885075ed5b963793b94cc0b70da8d6c3eea`，共同祖先为 `12db7bad76dc0af5da63ad4cfa8b07fef3c5e7cb`；6aeb 一侧有 28 个独有提交，main 一侧有 24 个。对双方都改动且内容不同的 20 个文件做隔离三方文本预检，11 个出现冲突，其余 9 个文本可合并也尚未通过语义确认。预检只读取精确源码，并把诊断写入独立 out；未合并分支或改动 main。审查 SHA256 `3322c240e5224d440aead22ce0f779b887be24e7a3af9f5e8dc9058f90661b91`。

| Extore 合同 | main304／cf | 6aeb／8e |
| --- | --- | --- |
| 固定依赖 | `packages/extore`，`cf3469dbe827afcfb2e3f054a63f7b9b7c40bc2b` | `third_party/extore`，`8e3063690cf5595884b1e639535869fd593e0296` |
| 执行方式 | 进程内 purego、专用 OS 线程、两份 SO，经 HTTPS 读取获准 catalog | 独立 `lmm-extore-host`、单份 `libextore_zplugin.so`、私有 Unix socket，共用既有 data_dir／UID |
| 配置来源 | `host_library`／`plugin_library`、两份 SHA、`ca_file` | `origin`／`host`／`library`／`sha256`／`socket`／`data_dir` |

11 个文本冲突分别位于 `.github/workflows/linux-packages.yml`、`config/extensions.example.toml`、`docs/extore.md`、`docs/progress.md`、`packaging/distribution.json`、`packaging/linux/build.py`、`packaging/linux/test_build.py`、`scripts/build-distribution.py`、`scripts/workflow-topology.test.mjs`、`services/extensions/internal/composition/runtime.go` 和 `services/extensions/internal/modules/store/extore.go`。同名 `store.extore_plugin` 配置承载两种不同合同，文本可合并的 `.gitmodules`、Dockerfile、configuration／run 入口也可能拼出不一致的运行方式。main 另有独立维护的 Cargo Git 来源、CLI gitlink、purego 依赖及 native commerce 输入，均在原 293／952 的源码闭包之外。下一步需为目标版本选定一致的 Extore 合同，逐项处理运行、配置、构建、包、工作流和测试，并保留另一维护线的改动及证据；本预检没有创建整合候选或提升发行指针。

预检同时发现 main304 已按原提交意图采用 canonical Git 依赖，源码发布校验却仍只接受 registry 字符串。版本 `0.1.21`／zconv `0.4.0` 本身一致，冲突在来源表示。最小修复已推送至独立分支 `review/source-release-git-304f49d8-20261121`：[cebc346c868e7d0c03bd82dab8d8f274ef7ec365](https://github.com/TokenNotIncluded/lmm-api-source/commit/cebc346c868e7d0c03bd82dab8d8f274ef7ec365)，parent 为 main304，tree 为 `77923ba1301d5be2e40b7df664b619d2dc4e9926`。仅修改 `scripts/source-release.py`、对应测试及 `docs/release-architecture.md` 的来源说明，其余 5,566 个叶节点和全部 20 gitlink 保持。

该修复保留精确 crates.io 版本、registry 来源与校验和校验；新增 Git 分支只接受 `version`、`git` 两个键：`version` 必须为 `=` 加请求的 zconv 版本，`git` 必须为 `https://github.com/LIghtJUNction/zconv.git`。锁文件必须唯一解析同版本 zconv，来源必须以 `git+https://github.com/LIghtJUNction/zconv.git#` 开头，后接完整 40 位小写十六进制提交，且不得出现 checksum；额外 branch／tag／rev／path／package 等键继续拒绝。原 request/schema/duplicate/semver/VERSION、唯一包及来源 override 门禁保留。

原四项测试逐字保留并通过，新增九项后完整纯元数据测试为 13/13 PASS、0 SKIP。使用精确 main304 的原 request、VERSION、manifest 与 lock，旧 `validate()` 实际报原来源限制，修正后返回原 `0.1.21`／`0.4.0`；全部验证仅使用本地元数据与临时夹具，外部进程／网络调用尝试为 0。验证收据 SHA256 `866df49bbcf8b5531fd0900f0ec2e3baf6dd80954ce40707e05f3cf9f1d5f4db`，独审 SHA256 `13a91de250ffbaf84651972a386a85cc0e6a1c43d402055fda3404756f7ff1c8`。2026-11-21 00:43:02 UTC 的发布读回核对了三文件全文、commit／ref 和冻结 tree，main 仍为 304；收据 SHA256 `b11bc62f825f713aa3450aabc121dd29cc40f671f2d6245ea60ff4531c5fdf53`。该独立修复未合入 main，未派发 CI 或运行源码发布器，不授予 main 部署资格，也未解决前述两种 ABI 的整合选择。

本轮在 main/cf 的 `cebc346c868e7d0c03bd82dab8d8f274ef7ec365` 基线上完成四文件修复，范围为 `packaging/linux/build.py`、对应测试及两个 canonical payment runner／test 文件。RPM 依赖按实际 OpenSSL 库及其符号版本声明；支付清单按两项业务测试和一项 hex codec 测试核对。main 原 GLIBC provider、musl／APK、双 SO 和 `allow_unversioned` 条件保持。

修复已提交为 [`7df35ca4185ccc67de70e3f68a7db7a01ca154c6`](https://github.com/TokenNotIncluded/lmm-api-source/commit/7df35ca4185ccc67de70e3f68a7db7a01ca154c6)，tree `4488ca69d1c93ae6a67e5b00733177057bee15e3`，parent 为 cebc，分支为 `review/main-cf-package-payment-cebc346c-20261121`。远端四个改动源文件全文及 5,569 叶整树读回一致，其余 5,565 叶和全部 20 个 gitlink 保持；回读收据 2,219 B，SHA256 `5a1bffc43df826df3e00e062e00ad6504fd0ebcd7c13c74c06a88d3a8b4a42d9`。

Libc 3 项、OpenSSL 5 项及 payment CaseSelection 4 项原方法实际全部通过：12 PASS／0 FAIL／0 ERROR／0 SKIP，外部执行及网络尝试为 0。原收据 38,398 B，SHA256 `ff7cc5e4cc578f375c48d66f013dffbd092eebac6ebf1a3c5d0dd91a8ec331a6`；实际结果的独立只读复核为 14,241 B，SHA256 `02558f0805408db531f4b035405b4615a614e8fdc0a165b3cf46ed577ad943ec`。验证覆盖纯解析与测试元数据，临时合成夹具已清理。

当前 main 指针仍为 304。main/cf 的下一步资格由最终选定源码自身的完整 CI、实际发行安装和目标主机检查取得；6aeb／816 的既有成绩继续绑定各自精确源码与运行组合。

原 `scripts/verify-deployment-ci.py` 要求最终 revision、`head_branch=main`、push／workflow_dispatch、最新 attempt 成功及唯一成功的 Implemented component checks。review 分支的既有或新 CI 均不替代最终 main 的部署准入。源发布还会重新检查当前 main、已验证源码和 tag 一致。main/cf、另一 Extore ABI 与正式发行指针没有自动合并或提升。

### 三、原运行容量、支付专项与浏览器已完成独立核验

运行包继续固定 `293e53f799b09f187893e9cdf0c3503973916676`。原 [44798787579](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44798787579) 的 Core、Web、CLI、Extensions 四类实际归档已核验。952 commerce 驱动另有独立来源，1,131 项编译输入及更宽运行输入分别检查；缺失 fixture 只在原 GHA 网络构建。原产物和旧失败不会改标成新候选。

原运行与 6aeb 的只读准入映射已冻结，审查 SHA256 `0e3aa8cb3560128ebd37e331ad464175002ee99db7e99191beff640d76f6f02d`。293→6aeb 的完整树差异限于已审查的七个叶节点，1660→6aeb 仅为 readelf 测试夹具；全部 20 gitlink 及 Extore8e 保持。输入清单的范围必须分别说明：

| 输入范围 | 已核对数量与差异 | 可支持的结论 |
| --- | --- | --- |
| 实际 952 commerce 编译清单 | 原保存清单 1,131 项，所列源码到 6aeb 无变化 | 支持该驱动所用源码相同；编译 revision、实际二进制和摘要仍属于 952 |
| commerce 运行来源集合 | 不执行选择器、按精确树静态归集 2,084 项，无变化 | 支持四组联合负载使用的所选产品／运行源码合同相同 |
| Core 资源输入集合 | 静态并集 902 项，无变化 | 支持 Core540 所选源码合同相同，原 prepared 与模型程序身份保留 |
| 支付来源集合 | 静态 2,096 项，恰有两处变化 | `tests/payments/verify_process.py` 与 `tests/payments/test_verify_process.py` 的元数据校正须以各自输入摘要记录 |

2,084／902／2,096 是从不可变 tracked tree 与原选择器计算的静态范围，不是新取得的运行收据。尤其 `payment.sources()` 包含整个 `tests/payments`，原 1,131 项 commerce 编译清单未包含这两项校正：Rust／Go 业务源码相同，不能据此把原支付输入清单称为与 6aeb 逐字一致。已审 canonical runner 与控制投影只差精确仓库根表达式；最终源码执行仍须产生包含两处元数据的新输入清单，原清单不能转签。控制器要求 `full_payment_acceptance=false`，selected payment302 的成功范围也不得扩大为完整支付验收。

816 本轮五组容量、支付专项和浏览器已实际通过，结果仍属于原 293 归档、952 驱动、816 控制器及该次 cgroup／fixture 环境，只能为已核对相同的产品源码提供组件证据。Browser16 的原 Web 文件图、固定8e fixture、工具及 cleanup 身份亦须保留。最终 main 自己的 canonical CI、选定可执行制品及输入摘要、真实主机 routes／TLS／配置／资源与启用检查仍须取得；相同源码不保证新构建字节相同，也不会生成最终主机的 PASS。

前三轮控制失败均已保留：[44820906438](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44820906438) 的固定 Extore gitlink 输入处理、[44822233959](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44822233959) 的官方 Core job 名称、[44823761920](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44823761920) 的支付测试旧清单，已分别通过精确来源修复。它们没有取得容量／支付／浏览器 PASS，skipped 不计成功。

第四轮 [44828479106](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44828479106)，控制 [550b2b00](https://github.com/TokenNotIncluded/lmm-api-source/commit/550b2b00e624c322cc292e52eec0eb065ddf52a9)，于 22:33:51 UTC 实际完成准备：原流程 553.074 秒、`source_drift=[]`、真实 full 3／ignored 1 支付清单原文及摘要已导出。3,131 B 收据 SHA256 `fa3386ffbbc2b800b847240a6bb6b5f46f86bdfe217480825bebdaf77e94eaf9`，结果明确为 `PREPARED_NOT_ACCEPTANCE`。

随后 runtime job `136612162997` 在 22:34:55 UTC 的宿主资源预检失败。114 B 日志报出 `Cannot set property DelegateControllers`，SHA256 `5ee0665f6e54dc1c38847d770d999f257138910fce07fbb3b89ae234f247b093`。此时尚未 restore，更未运行容量、长等待支付和浏览器。两份 raw 与两个小 artifact 各首次取得一次；299,735,709 B 大 prepared 包未下载、未独立核验 payload、未跨 run 复用。独立终态审查 SHA256 `aa2f817e48ec43f988babf5ba6f6a1ffa434c775b80bbb7b2bc7a123e27b7053`。

原脚本已使用正确的 `Delegate=cpu memory pids` 拼写。[systemd 255 参数编码](https://github.com/systemd/systemd/blob/v255/src/shared/bus-unit-util.c) 把列表映射为 DelegateControllers，[服务端实现](https://github.com/systemd/systemd/blob/v255/src/core/dbus-cgroup.c) 则仅允许新建 transient 单元设置该属性，现有 user@ 服务不能直接 set-property。

[单文件修复 1756f924](https://github.com/TokenNotIncluded/lmm-api-source/commit/1756f9240c34925c1ae72107e8150b1527b95248) 已推送：仅为当前一次性 GHA 用户排他创建 `/run/systemd/system/user@UID.service.d/90-lmm-final-runtime.conf`，保留原三 controller 的 Delegate 配置，经 daemon-reload 后继续原 linger／start。原双 bus、实际 controller／cgroup 读回和后续配额／进程归属检查逐字保留；无 restart／terminate、cgroup 权限改写或资源阈值变更。ERR 提示只输出行号和原退出码。修复后文件 3,505 B；bash 语法、隔离排他写入／拒绝覆盖／原退出码检查与独立静态审查通过。独审 SHA256 `309ed3b0134086855e1e8579c4af81aab3efeedd0ac6b6b96dcddefa065d4bda`。该静态审查随后由第五轮 GHA 的实际宿主预检补充验证；容量结果仍须后续运行取得。

第五轮 [44833392304](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44833392304) 绑定控制 1756、原 workflow 399255960、workflow_dispatch、attempt 1；单次 POST／HTTP 204 的收据 SHA256 为 `1551b54d0713ed07b9b0a9ff4d0556e44766586b1120828f78a4bdadd0974b5b`。prepare job `136623707902` 已 SUCCESS。runtime job `136628540976` 的实际 cgroup v2／双 manager 预检 SUCCESS：systemd 255.4-1ubuntu8.18、Delegate=yes、DelegateControllers=cpu memory pids，内核实际 controllers／subtree 均含三项。563 B 原预检日志 SHA256 `064c3094741349cada7ad81b5a11e9ab570a045d291d75dc43920f9a8439271f`。

随后原步骤 `Import verified original prepared bytes once` FAIL。232 B 失败收据仅报告 `prepared tar size or mode differs`，SHA256 `808a0ebed8e83482cd732eae8f0d96687377b5a9513bdeb1f6a636231e72f1a5`。五组容量、真实 302 秒支付和 16 case 浏览器均未开始；本轮不能记为这些门禁失败或成功，只记 NOT_RUN。整轮结论为 FAILURE，观察器于 23:12:24 自然结束，没有重启。

为定位组合错误条件，本轮独占首次取得 prepared artifact `14678226228`，299,737,154 B，ZIP SHA256 `92c101e983828b6a44aec1b7a106e07d981a3899a2bc22ae07086d3d9e368e66`。同 run／head／attempt、API digest、内 tar 和 manifest 摘要均匹配。只流式读取归档头与清单，未大范围解包、执行或复用二进制。733 个 tar 成员／732 个 payload 中，唯一不符是 `core-prepared/artifacts/model_process`：tar 与 manifest 同为 565,065,176 B（538.888 MiB）、权限同为 0500、普通文件类型正确；仅控制器新增的每成员 ≤512 MiB 条件为 false。原资源准备流程以精确编译输入及 artifact SHA 绑定该程序，没有这项传输尺寸限制。只读诊断 3,287 B，SHA256 `c2c882757c8cfef55736f744b61a215c41805f56f03df38de52c4e6564067c67`。

该传输问题已通过 [8164659d](https://github.com/TokenNotIncluded/lmm-api-source/commit/8164659d4f0d86fd8be9de5b648f1620f37d68a0) 的单文件控制修复提交：只有固定的 `core-prepared/artifacts/model_process` 可按连续编号、固定偏移拆为传输片，恢复时独占创建原路径，核对每片以及整文件 SHA／大小后恢复 0500。分片不落入原产物目录；其余大文件仍拒绝。单成员 512 MiB、总 2 GiB、10,000 项、原二进制字节、编译参数、原 prepared.json 及运行条件均保持。上一轮实际 565,065,176 B 文件对应 536,870,912 + 28,194,264 B；新一轮仍须按自身原清单核验。

本修复只修改 `package_payload` 与 `restore` 并加入三个传输辅助函数，32 个原函数及原恢复验证末段保持。30 项纯传输检查全部通过，覆盖缺片、重复、零长度、编号／偏移／顺序、摘要／权限、混入原文件、其他大文件与独占创建拒绝。纯测试用小样本和 16 B 分片，未冒称为实际大二进制验收。新控制源码 SHA256 `f1b91d6848320b3b90a4f667bb3c2261f5ee00f6f35785780215453612442ded`，独审 SHA256 `a9719cd97f25cd33ffa6a74f3fb27d11847b77afa513534c756c049f5883d60d`；parent 1756、tree `8e0ec8cdc8fa3235d474fc58e7a6ca680c4f2b71` 已完整读回。

第六轮 [44839545043](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44839545043) 于 2026-11-20 23:49:12 UTC 创建，绑定控制 816、原 workflow 399255960、workflow_dispatch、attempt 1。原 runtime293／commerce952 输入保持，单次 POST／HTTP 204 收据 SHA256 `2785c1b3d3c774312c8087797c0195e6a2abe3bbf412f7c153243afbbd9646d8`。prepare job `136644235064` 已 SUCCESS：原准备耗时 549.930 秒、`source_drift=[]`，3,135 B 原 preparation-result SHA256 `30ba4318b6aa807b49156737135e082c35715944239f30407647d6ad087197c9`，prepared 摘要 `98f34b9e5e11b16cf06a4d5e3a52c396519ea980adddda875667f1a366bff690`；结果仍明确为 PREPARED_NOT_ACCEPTANCE。模型与支付驱动的实际 compiler-artifact、full 3／ignored 1 清单及精确来源已核验，独审 SHA256 `048bc25c99d335c0b2c42091da3142b02debd008265d5f72d3c8ee832786efdf`。v33 已冻结的 2026-11-21 00:10:07 UTC 观察中，runtime job `136648321964` 的实际 cgroup／双 manager 预检与原 prepared 恢复步骤均 SUCCESS，原串行容量／支付／浏览器步骤 IN_PROGRESS；这确认真实跨机器恢复已越过旧失败点，但尚无单项运行结果可记 PASS。该快照中 runtime 原 raw 和结果收据尚待终态后首次取得，299,735,492 B 大 prepared artifact 尚未在开发机下载。 显式最终验收模式跳过的普通 protocol／market 不计 PASS；每个新终态 job 的原日志按原协议仅首次取得一次。

816 本轮原七项串行验收均已实际执行并通过：shared540、shared240、whole540、whole240、core540、payment302、browser16。原运行于 2026-11-21 01:01:02 UTC 的 seq64 记录 SUCCESS，runtime job `136648321964` 从 00:00:00 执行至 01:00:43 UTC；串行序列实际耗时 3,551.939 秒，failure 为 null，观察器于 01:01:04 UTC 自然结束。该显式验收模式中的普通 protocol／market 两个 SKIPPED job 仍不计 PASS。原 runtime raw 86,902 B、947 行，SHA256 `794721f2f573ec5ed35ac35bc52ae34826ddc95d74216bd6f7e6a234c47e0eef`。

| 原容量门禁 | 实际资源边界 | 本轮原始测量与结论 |
| --- | --- | --- |
| core540 | Core 单独 1 CPU／256 MiB，不含 PostgreSQL | 持续测量 540,723 ms，2,100 次模型成功；主负载峰值 18,333,696 B，PASS |
| shared540 | Core+Go 共 1 CPU／512 MiB，不含 PostgreSQL | 主窗口 541,038 ms，2,100 次模型成功、480 次商城完成并退款；产品组峰值 33,624,064 B，PASS |
| shared240 | 同上，120 个慢连接 | 压力至少 241,409 ms，120 个连接均有非零发送队列且恢复前未主动丢弃，PASS |
| whole540 | Core+Go+PostgreSQL 共 1 CPU／1 GiB | 主窗口 541,050 ms，2,100 次模型成功、480 次商城完成并退款；总组峰值 434,102,272 B，PASS |
| whole240 | 同上，120 个慢连接 | 压力至少 241,444 ms，120 个连接保留到恢复；总组峰值 348,778,496 B，PASS |

原 Unknown 保留计数也属于本轮结果：core540 的 `unknown_holds=2500`，shared540／shared240 为 `1924／120`，whole540／whole240 为 `1412／120`。这些计数没有被改写成已结清或零预留；容量结果保留这些 Unknown；真实财务材料仍待齐备。

这些资源边界由本轮实际 cgroup v2、user/system 两个 manager、`cpu memory pids` 委派和内核配额／采样记录核对。五项原资源记录中均未出现 OOM／OOM kill；whole 总预算覆盖 PostgreSQL，shared 与 Core-only 没有把 PostgreSQL 计入其限额。负载生成器、合成上游、宿主 OS、Docker daemon 与 Web 不在 whole 的 1 GiB 范围内，该预算的适用范围是上述被计量的进程组。Core 原结果还明确未验证真实 provider、旧架构对照和慢消费 socket 背压；后者由两项独立 240 秒组合场景覆盖。

payment302 的真实单调等待为 302,012 ms，62 次观察从 76 ms 到 302,012 ms，等待期间余额／凭证状态除 elapsed 外保持，尚无入账。原 Core 与 Go 均以不同 OS PID 真正重启、就绪后才首次入账；两次旧证明拒绝、一份 journal、一份 credit receipt、恰好一次 credit、原重放后资金不变及 reserved／unbalanced 均为 0 已核验。该 case 共 340.471 秒，原 result 为 PASS，`full_payment_acceptance=false`。验证范围为合成签名 bootstrap 与 Core／Go 恢复专项；完整支付资格、真实对账和财务导入按各自验收条件继续推进。

browser16 实际 16/16 PASS，页面异常和 observed_failures 均为空；536 个已记录响应中无 5xx，跨会话及撤销后的 409 为原预期拒绝。真实 Extore UI 先配置明确卖价，原连接导入 1 商品／2 规格／1 来源；九个导入后状态的映射 ID、商品／店铺／来源／规格及卖价均保持，重复 Sync、明确重叠 View、刷新和断连没有增加映射。访问令牌真实有效期恰为 900 秒，原 UI View 在到期后约 3.023 秒发出，新 access token 在到期后约 3.081 秒生成；原 refresh 使用计数 0→1、连接 version 1→2，未到期前及到期观测时 token 元数据与上游调用记录完全一致。跨会话 409 未消费原 flow，原会话正常刷新后完成导入；实际撤销后仍返回重新授权拒绝。

暗色默认值、现有 Theme 控件与 reload 持久化、桌面和 390 px 的界面边界均由原记录通过；本次独审没有额外做截图视觉批准。原封闭代理阻止了两次外部 snippet 请求，该事实保留。浏览器 launcher 实际 922.780 秒，browser／fixture 退出码均为 0；五个登记的业务运行进程全部停止，无登记残留，五个 driver 输入前后稳定。浏览器窄审 18,348 B，SHA256 `9c14c23ea8a2df82a1cdf065cf37855542b6b80a4aa24a275d0c20ffec31ccd8`。

结果 artifact `14683338970` 实际 ZIP 为 8,380,561 B，SHA256 `9f0b852f79dbf55b0aa523945fa5d342a2e585184b2150b893b65f614ee1ecec`，仅首次取得一次；397 个文件中 export-index 记录的 394 份导出均与原摘要一致，0 redactions，数据库／配置／凭据未上传。该次 299,735,492 B 大 prepared 包没有在开发机重复下载；真实 GHA 恢复了 732 个逻辑文件，原 prepared.json 字节未变，未复用准备 VM 的 PID 或 lease。

来源后验从 5,561 个普通文件及 20 gitlink 重建 tree `63454c389c91e7a202589bce8a43ff4a170d994b`，精确对应原产品 293；Extore 的 480 个普通文件及 1 gitlink 重建 tree `f82822091aa5a6a07eab5396e7af9253956f31b8`，精确对应 8e。四组 commerce 的各 2,084 项及支付 2,096 项实际来源摘要均无缺失、无不符，controller／compiled-inputs 前后稳定。Core 集群、四个 mixed 集群及产品组、两个 whole 组、支付集群均已清理；Core 两个 service unit 为 not-found，云端私有原 lease 于 01:00:33.775 UTC 释放，原 inode 保持、退出码 0。

完整独立汇总 41,381 B，SHA256 `64a4043187cafc3baafa85889ef0b8ab102fd5e423defd687fdfd640219cca39`。最终结论为 `PASS_WITH_EXPLICIT_SYNTHETIC_PREVIEW_SCOPE`，来源继续是 runtime293／commerce952／control816／Extore8e。最终 main/cf、真实财务和生产切流继续按各自来源与验收条件推进。

原运行要求仍是五组串行容量：Core 256 MiB／540 秒，Core+Go 512 MiB／540 与 240 秒，Core+Go+PostgreSQL 1 GiB／540 与 240 秒；原速率、120 个慢连接、P95／P99、账本断言与失败条件保持。之后支付专项必须真实等待至少 302 秒并完成 Core／Go 操作系统进程重启；Go reference 只能来自本轮实际成功的 shared540 收据。浏览器保留 16 case、900 秒真实 token 过期与原 1800 秒 launcher。prepare 95 分钟／runtime 150 分钟限额不变，旧测试成绩不转签本轮。

### 四、Windows 功能已通过，安装器体积仍不达标

产品 [3bdace68](https://github.com/TokenNotIncluded/lmm-scripts/commit/3bdace680e5fd4607517582104c13cfc95424295)、控制 [537c80cb](https://github.com/TokenNotIncluded/lmm-scripts/commit/537c80cbb66919b4ee6af9667e0a14494e668d33) 的原 [44823854144](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44823854144) 完整 SUCCESS。PS7、PS5.1 各 90 项原合同和 40 项 managed 回归全部 PASS；Pi、DSH、Codex、Claude Code、LMM 的实际首次安装与版本通过，DSH 插件列表包含 alpha.9。

21:56:53 UTC 的实际重复安装成功，21:56:54 UTC 的卸载及数据保留检查成功。原 shim、profile sentinel、provider TGZ 摘要检查与源码后验均通过，DSH_HOME、profiles、provider packages、pnpm、Python 和共享 PATH 保留。产品 76／控制 77 个叶节点及完整 tree、mode／type／blob、七份 raw 源码绑定前后一致。原 raw 75,646 B，SHA256 `12ffcd794aad6199f378e245155cb151b4af07f8f746d34d15b71d79ea5abfe1`；独立终态收据 SHA256 `8f5be214dffdad37a8406ff9eadb29a40d09c09329556a963003a57cf06cc927`。raw 和 13,412 B 源码 ZIP 各一次取得。

最终产品修复是在 JSON 结果收集处增加两个括号；旧 33 项回归保留，新实际文件用例覆盖单／双键、未知目录和非法 metadata 拒绝。此前原子替换空参数、Python 探针参数修复也通过本轮完整流程。历史 repeat 失败与当时未执行的卸载保持原记录。

原 23 路径合计仍 **72,912 B，严格 <50,000 B 为 FAIL**。至少需净减 22,913 B。相对原 a705，新增来自 DSH 持久安装 22,274 B、menu 665 B、Pi 2 B。有限设计调查仅量化 158 B 的可读局部去重，假设采用也仍为 72,754 B、还需净减 22,755 B；该设计没有应用、执行或发布，已通过的 Windows 产品不变。调查 SHA256 `9f218ee52a03c7385cddb348aa7630f8384fa5686c72eff0d2a8d0bb4eb939eb`。未删除功能、移出计数、混淆代码或放宽门槛，PR12／main／正式 installer 指针未提升。

2026-11-21 00:09 UTC 完成的整体只读研究进一步核验了全部 23 个计数来源及原预算函数。三份 DSH／CW／OC 主实现自身合计 51,160 B，已识别的 CW／OC npm／process 四块为 2,618 B。即使把这四块、CW／OC 四个入口和两份菜单的全部替代实现暂按 0 B 计，剩余仍为 58,719 B，距离最多允许的 49,999 B 还差 8,720 B。这是限定抽取范围的条件字节核算，不是删除方案或整个项目无法达标的证明。现有领域差异包括 DSH 路径身份／独占锁／venv／回滚、CW 交互登录／模型目录、OC release 校验／JSONC 配置事务；尚未建立保留全部行为及依赖前提的可读预算达标实现，未修改已通过 Windows 实际验收的产品，原体积门槛仍为 FAIL。完整审查 25,357 B，SHA256 `6fcd174598d1cd50371aff965c314df144efc586ae58a903f5fd20026afa5679`。

### 五、六个客户端版本已发布

| 插件 | 版本及 GitHub Release | 渠道／验收边界 |
| --- | --- | --- |
| Pi | [0.1.0-alpha.7](https://github.com/TokenNotIncluded/pi-lmm-provider/releases/tag/v0.1.0-alpha.7) | 原 npm OIDC 发布已核验；alpha 指向 alpha.7，latest 仍 alpha.2 |
| DSH | [0.1.0-alpha.9](https://github.com/TokenNotIncluded/dsh-lmm-provider/releases/tag/v0.1.0-alpha.9) | GitHub 资产已发布；不另称 npm 新发行 |
| CodeWhale | [0.1.0-alpha.4](https://github.com/TokenNotIncluded/codewhale-lmm-provider/releases/tag/v0.1.0-alpha.4) | GitHub 资产已发布 |
| VSCode | [0.1.6](https://github.com/TokenNotIncluded/vscode-lmm-provider/releases/tag/v0.1.6) | 42 项检查和实际 VSCode 1.104 激活通过；未宣称 Marketplace 发布 |
| Zed | [0.1.4](https://github.com/TokenNotIncluded/zed-lmm-provider/releases/tag/v0.1.4) | 29 项桥接／打包检查通过；未宣称真实编辑器验收 |
| OpenCode | [0.1.5](https://github.com/TokenNotIncluded/opencode-lmm-auth/releases/tag/v0.1.5) | 原九格矩阵 9/9、每格 52 项，共 468 项非跳过 PASS；三份实际资产已核验 |

六份源码身份与渠道记录继续保存在此前 [22:24 进度快照](https://github.com/TokenNotIncluded/lmm-api-source/blob/9dba972221219a18440fc0974c4f765d33e0b7bf/docs/progress.md)。既有 11 个 Go 插件 1.0.0 已审查，没有为改版本而重复发行。

### 六、生产仍需真实财务材料和最终主机检查

2026-11-21 00:54 UTC 再次只读复核原授权的 10 个精确路径：原审查、六份既有引用及三项材料。全部既有字节和缺失状态均与 2026-11-20 22:30 UTC 的记录一致；仅在这些授权路径范围内，三项阻塞仍在：真实 source_review 审批、完整钱包 reservations，以及实际初始化的 financial target。现有 identity-only target 不能替代财务目标。新复核收据 6,681 B，SHA256 `40c01b30d5165280c927991b8045de2e18223fd426749d252188f7ce9c319ae5`；私有清单只做流式哈希，未重新解析业务记录，live target 仍为 `NOT_CHECKED_OFFLINE`。raw/export 证据已解决，审批与预留不能由代码测试生成。

财务路径保持 Core69 导入后按原相邻版本升级，仅 available／debt opening，`all_financial_rights_complete=false`。后续须按原流程完成真实目标、备份恢复、writer fence／排空、未结负债、provider 和主机启用检查，再决定切流。当前证据不能承诺上线日期。本轮无生产连接、查询或写入，也未在开发机执行重型构建、依赖安装、PostgreSQL、产品二进制或容量负载。

## 历史快照（截至 2026-11-20 20:22 UTC）

**六个客户端插件的新版本已发布；服务器四种实际分发归档已核验，Web 完整 CI 已通过。** Core 全量数据库测试仍在运行。Linux 的 Arch 安装测试缺失依赖已经修复并进入原矩阵复验。Windows 持久安装器的 Pi 参数问题已修正，新的运行通过原 90 项 PowerShell 7 合同后，在新增受管安装回归中发现文件替换错误；完整 Windows 资格尚未取得。生产流量与数据库没有切换。

| 工作项 | 固定来源与原始运行 | 实际结果 |
| --- | --- | --- |
| 六个客户端插件 | 下文六份已公开 Release | Pi 0.1.0-alpha.7、DSH 0.1.0-alpha.9、Codewhale 0.1.0-alpha.4、VSCode 0.1.6、Zed 0.1.4、OpenCode 0.1.5 已交付；完整版本号与各渠道见历史清单 |
| 当前完整 CI | `952d21c5`；[44802025200](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44802025200) | Web、翻译、Market、Extore native 成功；Core 正在原 Rust／真实数据库测试步骤 |
| 原完整 CI | `293e53f7`；[44797641245](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44797641245) | 原 Web 失败保留；Core 继续原运行 |
| 实际分发归档 | `293e53f7`；[44798787579](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44798787579) | 5/5 jobs 成功；Core、Web、CLI、Extensions 的实际归档、清单、模式及摘要核验通过 |
| Linux 原生安装复验 | `749893fd`；[44807452120](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44807452120) | 两种架构均完成三产品构建，正在原生包／可选 GNU pair 构建；安装结果待终态 |
| Windows 持久产品 | 产品 `40c473f8`，控制 `4f047e02`；[44810237733](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44810237733) | 原 PS7 90 项 PASS，随后 managed 回归 FAIL；PS5.1 与真实五 CLI 未执行，体积门槛另为 FAIL |
| 容量、支付过期、真实浏览器 | 冻结的 `293e53f7` 实际运行组合；独立云端控制准备中 | 原入口与时限已确定，控制尚未派发，不能记作当前组合 PASS |

### Web 完整结果与仍在运行的 Core

[952d21c5](https://github.com/TokenNotIncluded/lmm-api-source/commit/952d21c5d5d849507478d8443f603c47e0126880) 只同步一份测试文件的四处已发布版本预期。新 Web job `136516335031` 已实际完成 format、copyright、lint、types、测试、build、bundle 与上传；原串行测试运行器完成 765 份源码测试和 6 份脚本测试。原 raw 1,656,287 B，SHA256 `0aabeb216590d694c1a4f24738fb1c3978fbe9af93e72b846b6245b4a50b0d6a`。Web 附件已生成，但未为重复验收另下载。

两个 Core job 均继续原运行。952 已通过 strict Clippy、all-target 编译，并导出 commerce 驱动附件；当前仍在 Rust／真实数据库测试，后续支付、Go 与 RPC 等步骤不能提前算成功。293 的旧 Web 失败没有被删除或改写。原时间上限、数据库负例与断言保持，未取消旧 Core 或重复派发相同全量测试。

### 实际运行组合与安装验证分开绑定来源

293 的四种分发归档均已取得真实文件。Core／Web／CLI 三份附件各首次下载一次，原 `build-distribution.verify` 检查通过；Web 全部 621 项文件含清单逐一核对。实际版本为 `0.1.0-preview`、epoch 为 1795200983，不沿用旧候选版本标签。Extensions 归档中的同一对 host／SO 已完成真实 native-flow，exit 0、11.9 秒、无 SKIP。

| 实际文件 | SHA256 |
| --- | --- |
| Core 可执行文件 | `ece6e14ff4772f3e6bf83095719c73ee62e3824075c0aefcdd4b79813b2f315d` |
| CLI 可执行文件 | `4ec8556d140c69eb913b0473ef9bc691b669bae8baf9c1ffa8df31cad01bd756` |
| Web 原归档 | `bfcef01dc4c480958e1ff34666c0a116e58350fb22a3bc812b0025a571a24968` |
| Extore host | `db346ba27cc7bdd121665c06cbaa5ff9099d4f4c6c6085cb6ae2cd1e91a5b125` |
| Extore plugin | `25c4928d879472c1f9ff11b9d7679875898a823c6cb042af7e59b510ef07980c` |

Core／Web／CLI 验收收据为 `a9c34974218de015be1d6e06603b8624c3b60108bc0791ebdeb5dd2d6ee2d9fd`，Extensions 收据为 `fe5b5fa2f62854b586be8f3aca0dd3f159c5d7833d61333e4004427829ae13cb`。原四份分发 job raw 捕获缺口继续保留，未重复下载，也未伪造补齐。

293→952 只修改 Web 测试，952→749 只修改 Arch 测试依赖。完整树及运行来源对应已核对。现有 preview／cutover 合同允许保留真实产物来源，并把独立测试结果绑定各自源码：因此这组运行包仍声明 source/runtime/expected-source 为 293，不会改标签成为 952 或 749。main／tag 的正式源码发行另有精确 revision 准入要求，不能借用该结论直接发布。合同收据 `c1f1c69390dd4679925d5da193caecfef193b988f9b185800083632d25e699ae`；运行来源对应收据 `e0b205e4a1db1d3234b0613f06ae54744aa98fc3b2baa0e7f71f39a422426422`。

### Arch 缺失 cmp 已最小修复，原 Linux 矩阵复验中

旧 [44798801506](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44798801506) 的 amd64 失败已经定位：原 Arch 容器 `/test.sh: line 133: cmp: command not found`。默认矩阵 6 项成功、Arch 失败；后续可选 GNU 安装步骤 SKIPPED，不能算 5 项通过。arm64 原 job 成功，实际默认 Debian／Ubuntu／Fedora／Rocky／Alpine 五项与 GNU Debian／Ubuntu／Fedora 三项均核实；实际 30 个包与 DEB 解出的同一对 native 文件核验通过。这不扩展到不存在的 arm64 Arch/openSUSE 或 GNU APK 项。

修复 [749893fd](https://github.com/TokenNotIncluded/lmm-api-source/commit/749893fdd19e90a82b94ab0e5cfba2730574970c) 的 parent 是 952，仅在 Arch 的 pacman 测试依赖中增加提供 cmp 的 `diffutils`，原比较断言保持。blob `83ea8b88225b16ff925a74e0cb45fdf776fcb24c`，文件 SHA256 `a2f816e4ce7d296ff1495094857f5f7d77833c58bdccda24e5fc89d0b19d42dc`。原 workflow 399255968 单次派发新独立分支，run 44807452120、attempt 1、20:00:54Z，保留原 40 分钟和两架构矩阵。此次 fixture 改动不另启动没有变化的 Core 全量 CI。

默认 Go-only／APK、main 祖先镜像准入与自动 archive deploy 的原有边界保持。native 包或归档通过不等于已经调整真实主机的 UID、data_dir、socket 权限并启用生产插件。

### Windows：已修正 Pi 分隔符，继续处理受管安装回归

第一轮控制 [74e0b645](https://github.com/TokenNotIncluded/lmm-scripts/commit/74e0b645ff0ca8a68ccc29f920d50b156f5e634a) 固定产品 ed10，实际 [44805027519](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44805027519) 在 Pi 参数合同失败。测试中的 npm.cmd 是 PowerShell 函数，裸 `--` 在函数调用中被解析为参数结束标记；保留原强断言后，产品 [40c473f8](https://github.com/TokenNotIncluded/lmm-scripts/commit/40c473f8c1a14412cbd17c832babf182802fd974) 仅把该操作数写成明确字符串 `'--'`，增加 2 B，其余 75 个产品叶节点不变。

新控制 [4f047e02](https://github.com/TokenNotIncluded/lmm-scripts/commit/4f047e0267bdc13a6fa7fe7a559aa88c80fb8458) 只更新产品 commit／tree、显式 Pi 原始字节绑定与说明；两个 checkout 的 autocrlf=false／eol=lf、原全部 Windows 命令和 15 分钟时限保留。工作流与助手经过原 actionlint 1.7.12，GitHub 两 blob 上传后完整回读，commit／tree／ref 精确核对。20:19:00Z 单次 POST/HTTP204，对应唯一 run 44810237733、job 136544630376。

实际 PS7 原 90 项合同已经 PASS，Pi 分隔符断言通过；随后新增 managed 回归在 `[IO.File]::Replace($temp,$Path,$null)` 报 `The path is empty` 后失败，正在核对 PowerShell 对 .NET 空 backup 参数的绑定。before／after 来源检查均成功。PS5.1、真实五 CLI、重复安装与卸载保留数据步骤仍未执行，不能转签为完整 Windows 通过。旧临时官方 wheel 的 394 秒 PASS 继续只覆盖原控制。

体积仍为 **72,902 B，原严格 <50,000 B 门槛 FAIL**，至少需减少 22,903 B。不提高阈值，也不把运行 helper 移出计数。产品仍在独立评审分支；PR12、main、服务器 installer 指针与正式发行未提前更新。

### 剩余云端验收已固定原入口

原五容量场景将针对上述真实 293 运行组合顺序执行：Core 256 MiB／540 秒；Core+Go 512 MiB／540 秒和 240 秒；含 PostgreSQL 的 1 GiB／540 秒和 240 秒。原速率、并发、慢连接数量、分位延迟、账本断言和 cgroup 读回保持，不另加负载变体。

之后执行普通 full CI 明确忽略的原 accepted-proof-expiry 专项：真实等待至少 302 秒、原 480 秒入口、原 OS 进程重启与账本断言。Go reference 只能来自本轮首个成功 shared540 的真实 acceptance.json，不能先造 PASS。最后重跑原 16 case 浏览器验收，保留真实 900 秒访问令牌过期、原 1800 秒 launcher，以及授权、导入、刷新、撤销、桌面／窄屏／深色路径。旧 120／6102 浏览器成绩不转签。

原 commerce 编译附件的真实 952 源码、编译输入子集和运行验证所需更宽输入集合分别核对；不能混称一个集合。缺失的 resource、payment 和 Extore HTTP fixture 仅在云端构建。控制准备与串行执行分开，单执行 runner 使用原门槛，首次必需失败即停止；当前仅形成计划与控制草稿，没有派发或产生当前组合的 PASS。本机未进行重型构建、依赖安装、PostgreSQL 或生产访问。

### 生产切流仍须真实财务和目标状态

财务只读 readiness 仍有三个材料根因：真实 source_review 审批、覆盖全部计划钱包的 reservations、实际初始化并正确声明的 financial target。raw/export 证明已解决。identity target 不能仅改名成 financial target，审批和预留金额不能代填。迁移范围仍为 Core69 导入后逐项原升级、available/debt opening，`all_financial_rights_complete=false`。

工程验收通过后还需真实备份恢复、写入围栏、在途写入清空、负债核对和目标主机切换条件。当前没有足够证据承诺生产切流日期；这批固定时长的技术验收也不能代替财务授权。本轮生产连接、查询、写入均为 0。下面 19:34 及更早快照保留其原结论。


## 历史快照（截至 2026-11-20 19:34 UTC）

**六个客户端插件的新版本均已发布；GNU x64 的 Extore 实际归档与 native-flow 验收已通过。** 服务器最新候选为 [952d21c5](https://github.com/TokenNotIncluded/lmm-api-source/commit/952d21c5d5d849507478d8443f603c47e0126880)，原完整 [CI 44802025200](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44802025200) 正在运行。它修复上一轮 Web 测试遗漏的版本预期；上一轮 Core 数据库测试继续保留运行。Windows 持久安装器已提交为独立草稿，体积门槛仍失败。本轮生产连接、查询、写入均为 0，没有切换生产流量。

| 交付项目 | 绑定的源码与原始运行 | 截止时状态 |
| --- | --- | --- |
| 六个客户端插件 | 下文六份已公开 Release | 已发布，渠道和宿主资格仍分别记录 |
| 完整服务器候选 | `952d21c5`；[44802025200](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44802025200) | 运行中；翻译已通过，其余按实际终态验收 |
| 上一轮完整 CI | `293e53f7`；[44797641245](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44797641245) | Web 失败，Core 原数据库测试继续；不能记为完整通过 |
| GNU x64 可选归档 | `293e53f7`；[44798787579](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44798787579) | 5/5 jobs 成功，实际归档与 native-flow 数据核对通过 |
| Linux 原生包 | `293e53f7`；[44798801506](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44798801506) | amd64 原默认包安装矩阵 FAIL；arm64 仍运行，完整安装资格未闭合 |
| Windows 持久安装器 | [ed10f7ec](https://github.com/TokenNotIncluded/lmm-scripts/commit/ed10f7ecf12946824717be84552dc9110b133c7e) | 独立评审草稿；预算 FAIL，Windows 产品实测 NOT_RUN |

### Web 原失败已修复，完整 CI 绑定新候选

原 Web job `136501247834` 的测试步骤实际运行约 668 秒后失败，不是超时。唯一失败文件为 `assistant-setup-tool.test.tsx`，其中 6 项通过、1 项失败。首个失败断言仍期待 Pi helper/provider 使用 `@latest`，实际界面已经输出已发布的 `0.1.0-alpha.7`。同一组合测试后面另有 DSH Unix、DSH Windows 与 Codewhale 三处旧版本预期，一并同步到当前命令。

修复只改这一份测试文件的四处字面预期，原 7 项测试与 106 条断言均保留；新正则匹配实际固定命令并拒绝旧浮动命令。DSH Web 固定 alpha.9 TGZ 并保留 `--profile web`、`--ignore-scripts`，Codewhale 固定 alpha.4 tag 和浅克隆。Desktop 与旧宿主的独立版本映射不变。当前树全部 771 份 Web test/spec 文件已检查来源，另一个相关 Forge 测试已经正确固定版本。没有以静态检查冒充 Bun/DOM 测试成功。

新 head `952d21c5d5d849507478d8443f603c47e0126880` 的 tree 为 `5c54b1576e61963e5baed635a4ccc22e8c9df637`，parent 为 `293e53f799b09f187893e9cdf0c3503973916676`；after blob 为 `1ce374535cd8fd9d8ddc02db4d86f84eedb4c0af`。独立分支 `review/core-public-providers-20261120-web-pin` 避免原 concurrency 取消正在执行的上一轮 Core。原 workflow 399255954 于 19:25:41Z 单次 POST/HTTP204，唯一 run 44802025200、attempt 1、created 19:25:43Z，翻译基础仍为清理后的 `987e4d3b`。原超时、守卫和断言未降低；两轮结果分别绑定各自源码。

原 Web raw 仅下载一次并已保存，SHA256 `17853b58c39799968c7880c4c895bf3c75eaa2ae9cfeff4790f428e2521e3e67`。修复 manifest 为 `bc35db53319252b9ea8ba4e3c452ffcd6d17e9f6b56e0ecafc3fe90a406067c2`，新 CI dispatch 回执为 `03c18fbed6c1074b7bf8b1058ad7b2c16ce6bbb685182547f0534eb5068adc52`。

### Extore 归档已取得真实产物证据

原 distribution run 44798787579 的五个 job 全部成功。实际 Extensions 归档和 native-flow evidence 两份附件各下载一次，均匹配 GitHub digest。归档精确包含 19 个文件，清单、尺寸、模式、摘要和目标架构核对通过；32 项数据绑定检查全部通过。实际 `lmm-extore-host` 与 `libextore_zplugin.so` 的摘要，与真正执行 native-flow 的文件逐项相同。该测试 exit 0、11.9 秒、PASS，无 SKIP；23 份 receipt 源文件还逐项对应冻结完整树的 Git blob，71 份 Extore 源文件已核验。

- 实际归档 SHA256：`38a9c81b0032721f6cf44c4c64b099bd0f8f6f19112fa638948526f383e7f7a2`。
- host SHA256：`db346ba27cc7bdd121665c06cbaa5ff9099d4f4c6c6085cb6ae2cd1e91a5b125`。
- plugin SHA256：`25c4928d879472c1f9ff11b9d7679875898a823c6cb042af7e59b510ef07980c`。
- 产物验收回执 SHA256：`fe5b5fa2f62854b586be8f3aca0dd3f159c5d7833d61333e4004427829ae13cb`。

四个已完成 job 的首次 raw 捕获因 gh CLI 检出终端转义序列而失败，stdout 为 0 字节；原失败回执和 stderr 保留，没有重复下载。这四份 raw 缺口未抹去。上述结论依据原 GitHub job/step 终态以及实际归档和 native-flow 附件，不补造缺失日志。Core raw 的首次捕获成功。归档资格只覆盖原 `293e` 的 GNU x64 交付，不自动成为新 head、Linux 安装矩阵、main-only 镜像发布或生产激活资格。

原 native run 44798801506 保留原 40 分钟限时。amd64 已通过可选 GNU pair 打包、实际 DEB 解出的 pair native-flow，随后在原默认包容器安装/升级/运行/移除矩阵失败；job 136505244352 的唯一 failed step 为该安装矩阵。arm64 仍在相同原步骤运行。截止时具体错误日志待读取，尚未认定根因或重派流水线。自动 archive deploy 入口保持 Go-only；插件版切槽仍需满足受控路径、共同 UID、数据目录与 socket 权限，未绕过 main 祖先发布准入。

### Windows 产品草稿已提交，预算与实测分别记录

持久安装器 v2 已冻结为完整 tree `dc526be6e7b51fd0447867b29e1b3692b8a2ba68`，六个文件通过 GitHub blob 回读，提交 [ed10f7ec](https://github.com/TokenNotIncluded/lmm-scripts/commit/ed10f7ecf12946824717be84552dc9110b133c7e) 的 parent 为 `a705fc74`，位于独立分支 `review/dsh-wheel-persistent-20261120`。提交明确标注 DRAFT；PR12、main 与 release 未更新。

实现使用完整官方 wheel、固定版本 venv、受管 shim 和安装回执；在写入之前检查真实 profile 路径与受管目录关系，在首次删除之前预检所有受管目录与回执。原 v1 的 profile 随卸载删除、遇 junction 前先删除 shim 两个静态发现已在 v2 修复，并经独立静态复核。非 x64 原路径、CW/OC 文件和原 90 项 PowerShell 断言保留。Windows 路径、junction 与删除行为仍待实际 Windows 运行；不把静态审查写成运行通过。

原严格预算用例真实结果仍为 `72900 not less than 50000`，exit 1；满足原 `<50000` 门槛至少需减少 22901 字节。没有提高阈值或把 helper 移出计数。默认远程 menu 路线继续拒绝未固定版本的 DSH 安装，待实际审核版本发布后再固定。下一步是在独立控制工作流中 checkout 精确产品 commit，保留原 15 分钟、PS7/PS5.1 各 90 项、五个 CLI，以及新增受管路径、重复安装和卸载数据保留检查。控制源码尚在准备，截止本节没有派发。

先前官方完整 wheel 临时控制 run 44786638207 的 PASS 继续单独有效，不能转写为此持久产品通过。v2 manifest SHA256 为 `0af9d36166918ca06ae9631d7f282ab3aa9025f3fddb27a18bdffe6af6249617`，独立复核为 `704d333914a21898ed181aacdaaa06a50c1c1b6f8e5a5fe48f30d6beb0fb0f68`。

### 生产承载仍有明确前置条件

工程侧继续完成当前候选的原完整验收、实际原生包安装验收与必要的候选容量验证，现有旧源码成绩不会直接转签。财务只读 readiness 仍为原 5 个 BLOCKED、三个根因：真实 `source_review` 审批、全部计划钱包的 `reservations`、实际建立且正确声明的 financial target。raw/export 证明已解决；identity target 不能仅改名充当 financial target，审批与预留金额不能代填。

当前迁移范围仍是 Core69 导入后原逐项升级、仅 available/debt opening；`all_financial_rights_complete=false`。工程门槛和真实财务材料闭合前，没有可据以承诺的生产切流日期。以下 19:02 及更早快照保留其原结论和证据范围。


## 历史快照（截至 2026-11-20 19:02 UTC）

**六个客户端插件的新版本均已公开；服务器正式交付候选 [293e53f7](https://github.com/TokenNotIncluded/lmm-api-source/commit/293e53f799b09f187893e9cdf0c3503973916676) 已推送，并启动原完整 [CI 44797641245](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44797641245)。** 原四个失败用例已定向通过，当前完整组合仍须等待实际终态。Windows 官方完整 wheel 路线已在原限时内通过，但持久产品草稿的原体积预算失败，数据保留修复仍在独立审查。服务器候选保留既有 installer 指针，单独推进交付验收。本轮没有迁移生产数据库或切换生产流量。

### 已公开的插件版本

六份 GitHub Release 的实际下载附件均已核对字节数与 SHA256，并绑定原构建产物和固定源码。Pi、DSH、Codewhale 为 prerelease；VSCode、Zed、OpenCode 为稳定 GitHub Release。编辑器市场、npm 渠道与真实 Core OAuth／付费推理链分别验收。

| 插件 | 已公开版本 | 固定发布源码 | GitHub Release ID |
| --- | --- | --- | --- |
| Pi | [0.1.0-alpha.7](https://github.com/TokenNotIncluded/pi-lmm-provider/releases/tag/v0.1.0-alpha.7) | [b5eb6321](https://github.com/TokenNotIncluded/pi-lmm-provider/commit/b5eb63210611ddb1bc0ca8255b87a13905212e62) | 452761984 |
| DSH | [0.1.0-alpha.9](https://github.com/TokenNotIncluded/dsh-lmm-provider/releases/tag/v0.1.0-alpha.9) | [19489729](https://github.com/TokenNotIncluded/dsh-lmm-provider/commit/19489729983cbe3c6225816d2594af819ba54b7f) | 452763949 |
| Codewhale | [0.1.0-alpha.4](https://github.com/TokenNotIncluded/codewhale-lmm-provider/releases/tag/v0.1.0-alpha.4) | [e34e356a](https://github.com/TokenNotIncluded/codewhale-lmm-provider/commit/e34e356ab7e815b24e89bab05688b3404d2cc638) | 452774919 |
| VSCode | [0.1.6](https://github.com/TokenNotIncluded/vscode-lmm-provider/releases/tag/v0.1.6) | [fc0fed00](https://github.com/TokenNotIncluded/vscode-lmm-provider/commit/fc0fed00fcc2acfd5c347f0c9466a1c5a591bf55) | 452786332 |
| Zed | [0.1.4](https://github.com/TokenNotIncluded/zed-lmm-provider/releases/tag/v0.1.4) | [5bef2a06](https://github.com/TokenNotIncluded/zed-lmm-provider/commit/5bef2a061f19788bd112718a14d5b587fe867358) | 452787284 |
| OpenCode | [0.1.5](https://github.com/TokenNotIncluded/opencode-lmm-auth/releases/tag/v0.1.5) | [c32312c8](https://github.com/TokenNotIncluded/opencode-lmm-auth/commit/c32312c8d1e03c28e1204d33e7e94d1b312ac832) | 452981260 |

Pi 的[原 npm OIDC 发布](https://github.com/TokenNotIncluded/pi-lmm-provider/actions/runs/44753390279)实际完成 publish 与 registry readback。`alpha` 指向 `0.1.0-alpha.7`，`latest` 仍为 `0.1.0-alpha.2`；30 个包内文件、TGZ 摘要及 SRI 已核对。控制源码 `198ba195` 与发布源码 `b5eb6321` 分别记录；并发 main 更新未改变原发布工作流或脚本。attestation 元数据 URL 已存在，本轮未另外执行密码学验证。DSH alpha.9 已公开 GitHub 附件，尚不能称为 npm 新版或 GitHub `latest`。

DSH 五宿主、Codewhale 四组单元／三宿主／两组 Windows 的实际结果保留。[原 editor-providers](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44744756613)完成 VSCode 42 项测试和真实 VSCode 1.104 激活，以及 Zed 29 项测试与打包；Zed 尚无真实编辑器启动验收。这些证据不等于生产 Core 端到端验收。

### Windows DSH：官方完整安装包路线实际通过

[安装器 PR #12](https://github.com/TokenNotIncluded/lmm-scripts/pull/12) 当前源码 [abde4ef2](https://github.com/TokenNotIncluded/lmm-scripts/commit/abde4ef2bdd5f9f42cf5d4397d9109fab49fbcd1) 已修正 macOS 同目录别名及 PS5.1 模块 mock 夹具；产品摘要保护、原断言和预算保留。[Codewhale 三行](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44758076810)与 [OpenCode 六行](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44758077025)全部成功；[Installer checks](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44758076847)四 job 成功、Windows job 取消。Windows 的 PS7、PS5.1 各 90 项实际通过，Pi 安装完成；DSH 未完成，后续 CLI 与最终版本检查未执行，不能把该行记为通过。

仅增加阶段标记的 [DSH 控制](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44768274211)保留相同产品字节、npm 参数和 15 分钟 job 预算。实际停在 `npm.cmd install -g @deepseek-ai/dsh@latest pnpm@latest`，host-install 经 636,824 ms 以 exception 结束；附件下载、摘要检查、plugin-add 三阶段均未开始。运行最终取消；没有足够证据把取消归因于具体系统事件。后继 [7417d563](https://github.com/TokenNotIncluded/lmm-scripts/commit/7417d56372dcbe0b90777f8bb1c6bef1fc5529b4)只在该命令作用域打开 npm timing／info，并输出白名单阶段信息，保留原参数、退出码、生命周期脚本、并发行为和预算。[本次观察运行](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44774378194)亦最终取消，job 用时 907 秒；PS7、PS5.1 各 90 项仍实际通过。npm 依赖树解析 15.562 秒完成，随后停在取包／物化阶段，未出现 lifecycle 开始或 reify unpack／build 完成记录。白名单匹配到 1,020 条可见 GET 200，其中 340 条计时超过 60 秒、32 条超过 300 秒；实际 DSH 归档为 0.2.2-rc.5，该条计时 652,064 ms，pnpm 版本未观测。取消时读流收到 KeyboardInterrupt，npm 退出码未知。已按 npm-registry-fetch 19.1.1 精确来源确认，这个计时从请求开始到响应体 end，包含消费者／缓存及背压影响；白名单未记录 cache-only 或带 attempt 后缀的全部格式。因此不能把这些记录概括为全程没有重试／错误，也不能断言网络、文件系统或并发为唯一原因。没有重试同一失败运行。后续 [fe016cd2 官方完整 wheel 控制](https://github.com/TokenNotIncluded/lmm-scripts/actions/runs/44786638207)已实际通过。唯一 job 于 17:47:30–17:54:04 UTC 运行 394 秒，原 15 分钟限时、PS7／PS5.1 各 90 项、五个 CLI 安装顺序及最终版本检查均保留。该控制只改变两份控制文件，原 74 个产品文件不变；不能把它的成功转写为 abde 默认 npm 路径已经通过。

实际下载官方 `deepseek_harness_runtime_bin-0.2.2rc2-py3-none-win_amd64.whl`，86,959,936 B，SHA256 `a791af95a97be205a73354c6819e95b5fe2ff40a57ef8a78d53c188ee33e9aea`。在确认 Python ≥3.10、平台、入口及实际无 Requires-Dist 后，通过独占 venv 正常安装完整 wheel；不解析依赖、不升级 pip，也不抽取孤立 EXE。包内主程序、ripgrep 和下载清单三项安装后的实际摘要均与原 wheel 一致。

| 实际阶段 | 结果与耗时 |
| --- | --- |
| 创建 venv | 退出 0，6.450 秒 |
| 完整 wheel 安装 | 退出 0，1.711 秒 |
| 原 pnpm 安装 | 退出 0，41.980 秒 |
| 宿主全流程 | 退出 0，55.252 秒；实际 DSH 0.2.2-rc.2 |
| 原 provider 安装 | 退出 0，2.957 秒；原 URL、摘要、profile 和 ignore-scripts 参数保留 |
| 真实 Web 插件 | 未认证 401；认证后 200、signedIn=false；未启动 OAuth 或模型请求 |
| 自建 Web 清理 | 仅该 PID 树，taskkill 退出 0；被强制停止的 wrapper 退出 1 与清理方式一致 |

最后五个 CLI 均实际执行版本检查：Pi 1.6.0、DSH 0.2.2-rc.2、Codex 0.171.0、Claude Code 2.1.329、LMM 0.1.0。原始日志只下载一次，48,196 B／765 行，SHA256 `d7ecc46872751637ec65a2ca4cf6c4bcdcd79409bb4685c3685b3842c92f60f7`。临时工作区随后重建；已从原先读取的内存载荷将这份日志逐字恢复到开发机，没有再次下载或重跑。丢失前生成的终态收据为 14,079 B，SHA256 `261aafa1a4fe7e6510fd1be3a994173530121e7d0e5db32870ff19c864501c4f`；其原文件的丢失与恢复记录分别注明，未伪称原路径仍存在。

这证明官方完整 wheel 控制方案可用，不能转签持久安装器资格。原临时日志已恢复到开发机只读文件，恢复收据 SHA256 `6091867758683ae5556e6730f9f8d626703fa726755434c56aa4dc40799ce177`；根再次核对原日志 48,196 B 与摘要一致，没有重复下载。

Pi alpha.7 四文件稿已提交独立分支 [a705fc74](https://github.com/TokenNotIncluded/lmm-scripts/commit/a705fc742686463a087905a8a21b48900a118c38)，75 个 leaf 中仅四项改变，23 个脚本共 49,971 B；原六项 Bash 已通过，PowerShell 及最终组合仍待。该分支没有创建 PR 或触发 CI，PR12 保持 abde。

新的持久 Windows v1 草稿包含完整固定 wheel、最终路径 venv、受管 `dsh.cmd`、PATH 归属、重复安装、失败恢复与卸载。已补齐全部 23 份预算源码并逐 Git blob 核对，执行未修改的原 `InstallTests.test_flat_layout_and_size_budget`，实际结果为 **68,563 not less than 50,000，exit 1**。较 a705 增加 18,592 B；满足严格小于 50,000 至少还需减少 18,564 B。未提高预算或把运行 helper 移出统计，CW／OpenCode 及 dsh.sh 保持原字节。

独立静态审查确认 v1 存在两个需要修复的问题：接受位于受管 runtime 内的 DSH_HOME 后，卸载可能一并删除该目录；包含 junction 的 runtime 会在卸载拒绝前先失去命令 shim。修正正在独立 v2 中处理，同时说明硬中断遗留未知目录不由安装器盲目清理。v1 清单摘要 `e398bc1aa996ce3a7175b71c5926efe43c0c7f3a171c690943eaaa83c22ab9d2`，审查回执摘要 `54c2f430b7e9d970d77c3b6a37d06af9f410808b2db696ebf62bc90b0779f03b`。本产品没有执行新 PowerShell／Windows 安装验收，未推送、未修改 PR12。原 90 项断言保留不等于新默认路线已通过。

### OpenCode 0.1.5 已正式发布

[OpenCode 0.1.5](https://github.com/TokenNotIncluded/opencode-lmm-auth/releases/tag/v0.1.5) 已于 17:21:30 UTC 公开为稳定 GitHub Release，ID 452981260，`latest` 已指向此版。tag、`target_commitish`、发行清单及实际包均绑定最终主分支 [c32312c8](https://github.com/TokenNotIncluded/opencode-lmm-auth/commit/c32312c8d1e03c28e1204d33e7e94d1b312ac832)，tree `03c277a594490eb0a1e0e09070530c1ebc98d8bd`。主分支采用正常双父合并，保留并行的 `DYNAMIC_CATALOG.md` 目录说明更新。

[729 候选九行](https://github.com/TokenNotIncluded/opencode-lmm-auth/actions/runs/44778448175)和[最终主分支九行](https://github.com/TokenNotIncluded/opencode-lmm-auth/actions/runs/44779914524)各自原始运行均通过。最终源码的 Windows／macOS／Ubuntu × OpenCode 1.18.34、1.19.6、latest 九组，每行实际 52 PASS、0 skipped／failed／cancelled，共 468 PASS；原 build、dist 守卫、typecheck、pack 检查、真实 host smoke 和三项 integration 均有完整日志。`latest` 实际解析版本未由原日志输出，未猜测。Windows 原七次 ACL 调用和 lost-ACK 断言完整保留；最终三个 Windows 行的 journal 用例约 1.10–1.14 秒，仍使用原 5 秒预算。

实际 `npm pack` 仅执行一次，原 prepack build 与产品 helper 校验均成功，耗时 3.399 秒。63 项源码前后逐字相同；包内 45 个普通文件、解包 359,563 B 全部匹配固定源码。真实 TGZ 为 107,174 B，SHA256 `32662d8f6e4b5e7c723b5079decb84113305622b773836c5bd8ffb7bbf163e66`。发布同时附带六字段 `release.json` 和单行 `SHA256SUMS`，安装器据此核对 commit-addressed 文件名、固定源码和包摘要。首次后置审计误将 npm lifecycle banner 只查 stdout，而实际 banner 位于 stderr；修正只读取原两条输出与既有 TGZ，未重打包或修改源码、dist、包字节。

发布器实际完成草稿创建、三附件上传、逐附件下载字节核验、正式公开及 tag／target／latest 回读；收据 SHA256 `d3bf8202e86222307aad0ba015d0cd6bdd5eb86c9d070f167e64e3a6658ea74d`。本轮没有执行 OpenCode npm publish。

原产品 helper 来自[真实 Windows 构建](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44765614413)，EXE 14,848 B，SHA256 `cc9b0438f27ae56bcaa52374298c3c2ef4b86e36452efd9bca972c4a92c5581a`，已审 C# 存储逻辑及原 5,000 ms／4,096 字节边界保留。[独立测试 oracle 构建](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44776775599)实际编译出 9,216 B 程序并通过原 52 项，验证了源码／EXE／收据绑定及两类进程无遗留；测试程序未进入发行包。先前产品分帧失败与 [e1f 独立 PowerShell oracle 超时](https://github.com/TokenNotIncluded/opencode-lmm-auth/actions/runs/44770952301)的原始记录均保留，未把没有 Add-Type 的测试工具误写成动态 C# 编译超时。

上述宿主集成检查使用本地 OAuth 夹具，验证 PKCE 重连与明确协议拒绝；没有执行真实付费模型请求。unsupported legacy inference 仍明确拒绝，不扩大为生产 Core 登录／计费全链资格。

### Core 四个失败已定向修正

公共客户端 component78 的[普通定向验收](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44737269961)已实际通过三项 schema／升级和六项 HTTP 用例。后续 [a1250080 完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44747762596)最终失败于三个离线后缀升级用例及 community 历史导入；strict Clippy、all-target 编译与其他四个 job 成功。原始日志和失败证据保留。

[a2b2c537](https://github.com/TokenNotIncluded/lmm-api-source/commit/a2b2c53732e93871d2bd6f26c78409f844ac9217)在 Python 导入入口注册已实际冻结的 component78 目录，保留旧版本与错误目录负例；原五项 Python 测试通过。SQL 仅改注释，数据库逻辑未变。完整源码上的原 docs-only 与该文档显式链接检查通过；首个命令被嵌套 mise 配置阻止后，使用既有系统 Python 执行，未修改信任配置。

[73711f0e](https://github.com/TokenNotIncluded/lmm-api-source/commit/73711f0ebc2a6475198355d3cac3e0b43eb41d1b)在一次性 CI PostgreSQL 服务中关闭 autovacuum，并以新连接确认 `off|f`；六个原离线阶段在原 5 秒总预算内检查完整目标数据库会话与 prepared transactions，保留生产 fence、原六次 CLI 参数、成功断言及拒绝负例。

[四原用例复验](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44773426667)已全部成功，各执行一次、0 ignored：channel observations 8.86 s、navigation options 18.97 s、trust policy 8.99 s、community connection history 7.22 s。四个测试执行文件来自本次真实 Cargo 编译；2,395 项源输入前后逐字相等，全部 5,593 个 Git tree 叶节点核对一致。PG 服务为 18.6、客户端工具为 18.7；保留的 early58／`2a0c7bd3` 执行器使用原 Rust 1.99。community 原 Rust 断言同时要求 Go 子命令退出 0 并含 `TestConnectionHistoryFunnelLive` 的 PASS；私有 Go 子日志未上传，未宣称直接审阅这些日志。

独立收据 SHA256 `829e85d0722c0b36124f2248a2018b14ef63fb63f987dae9a1cf09cc37d23102`。原 navigation 的 SQLx Pool 提示和数据库负例 ERROR 保留；GitHub 实际清理了该次 PG 容器与网络。该定向模式明确跳过完整 protocol／market job，不能替代最终组合的 full CI，也不能证明旧拒绝瞬间的唯一原因。历史 bbf 的[另外三个原升级用例](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44764024336)亦各实际通过一次，沿用其独立源码与证据身份。

### 安装指南与已发布插件指针已提交

[4706da93](https://github.com/TokenNotIncluded/lmm-api-source/commit/4706da93e280876f8ff9e6a54245fa265a1aef24)在 73711 基线上更新 11 个指南／翻译／测试文件，以及 Pi、Codewhale、VSCode、Zed、OpenCode 五个已发布源码指针。完整 tree 为 `563f21b62fc98725536ea2389ad97e80bafca064`，5,593 个 leaf 核对一致；DSH 保持 19489729，Extore 保持 8e306369，未合格的 installer 指针未提前替换。

Pi helper 与 provider 操作数均固定 alpha.7；Codewhale 固定公开 alpha.4 tag；DSH Web 使用 GitHub alpha.9 包，Desktop 保持 alpha.4 与旧宿主 alpha.3 映射。14 个原导出名称保留。七份 locale 各新增六键，原 13,430 个键及所有旧行逐字保留；原翻译 writer、sync 和原回归函数通过。原 Forge 七项及新增版本入口回归合计 8 PASS、0 skip。实际 formatter、完整 Web format／lint／types／build 仍留原最终 CI，未用局部测试替代。

后续清理后的指南基础为 [1aa53b4a](https://github.com/TokenNotIncluded/lmm-api-source/commit/1aa53b4a3e4d0fea0a36d83586f0fef8fdbc20c0)，相对 4706 恰好只删除 12 张生成的审阅 PNG；其它 blob、mode 和全部 20 个 gitlink 一致，清理结果没有回填。对应 translation base 为 `987e4d3bedb4c285ec1b4d4943524d19eb6b9c73`，与 73711 也只相差这些图片，七份 locale 基线完全相同。原固定历史执行器仍可由新历史获取。

### 可选 Extore 正式交付已提交候选

原 73711 的归档和 Extensions 镜像只交付 Go 程序；debug native CI 也不构成正式产物资格。现已提交 [293e53f7](https://github.com/TokenNotIncluded/lmm-api-source/commit/293e53f799b09f187893e9cdf0c3503973916676)，parent 为清理后的 1aa，完整 tree `63454c389c91e7a202589bce8a43ff4a170d994b`。完整树由全部 5,581 个 leaf 重建核对，12 个既有文件仅改变内容，20 个 gitlink 和原文件模式均保留；installer 仍为 `452efac0f1698676060605f10a9e4705b487e6ba`，Extore 仍为 `8e3063690cf5595884b1e639535869fd593e0296`。

可选 GNU 归档、原生包、runtime-extore 镜像成对交付 `lmm-extore-host` 与 `libextore_zplugin.so`，绑定实际源码、gitlink、架构和摘要，保留 Go-only 默认及原 APK 路径。原 distribution/native 工作流将对实际归档或 DEB 解出的同一对文件运行 native-flow，并验证运行库及安装／升级／移除后的文件。两处新增 mise 步骤固定 2026.11.10，并分别固定 x64、arm64 实际二进制摘要。

发布工作流中同一步写入 GITHUB_ENV 后立即读取 PREVIEW_TAG 的错误已修正：先显式 export 来源与 tag，再写后续步骤环境。实际提取的旧 Load step 三种组合均复现 KeyError；新版本原 17 项加新增 2 项通过，错误 tag 仍拒绝。这些检查使用 Git／Docker 替身，不是镜像运行验收。归档／原生转换原 19 项、拓扑原检查、固定 actionlint 与 docs-only 的轻量结果分别保留；没有在开发机重编译、安装或启动数据库。

自动 archive `deploy.yml` 完整保持原字节。独立消费者审查发现原 a/b 槽位 symlink 与 Go／Rust 的受控路径要求不兼容，外部 TOML 不随切槽改变，现有 verify 仅检 Go 服务；因此撤下草稿中尚不成立的 Extore 自动部署选项，保留原加载检查。成对代码交付不自动创建实时数据库、调整共同 UID 或启用插件。原生包和镜像也需满足真实安装路径、同源摘要、同 UID、共享实时 data_dir、0700 目录及 0600 socket 等启用合同。

最终 v3 补丁 SHA256 `0c3471ebe82c501d01f7018a4c8aa0aa1820f3ba30093cf45b50f848ba05139e`；完整组合收据 `6296c3492f07f86aef2f8e47123ce3dd9557737b85f952163964f5a915dca873`；部署消费者审查 `015c4055542be4ae6e1df289aed003bad11b2c03bc94f30191e117bee563cb4d`。distribution 和 Linux 包原入口允许 review 候选；GHCR images 原有 main 祖先准入不变，本轮没有将 review 代码绕过该门槛发布。

### 最终组合与生产上线

服务器候选的原完整 [CI 44797641245](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44797641245) 于 18:57:34 UTC 创建，唯一 dispatch HTTP 204／POST 1，源码固定 293e53f7，translation base 固定 cleaned 987e。翻译以及 Web 的格式、版权、lint／类型检查已通过；其它阶段及整个 run 的最终结论尚待独立回读。Web 快速门槛完成后，将在同一源码运行原可选 distribution／Linux package 验收，保留原限时和拒绝条件。

正式 Windows 安装器在独立分支草稿中继续审查，其体积门槛不阻塞服务器现有 installer 指针的 CI。当前组合仍需完整 Core／market、Web、native integration、真实交付产物及安装结果。旧 22c、旧浏览器和 120e0401 容量结果只绑定各自输入，不能迁移为当前组合的生产资格。

已完成的只读上线审计将实际财务 readiness 的五个 BLOCKED 收敛到三个根因：

1. **本批 source_review 缺失。** 需要真实审核结论，绑定 source archive、mapping、identity map 与经济证据摘要，保持 500,000 credits/USD 及其他权利暂不启用的原合同；不能由代码生成一次摘要就充当批准。
2. **完整 reservations 缺失。** 需要覆盖计划中全部钱包用户、无重复且额度合法的预留声明，并绑定已审核计划和证据；部分短信预留事实不能代替完整声明，也不能擅自填全零。
3. **真实财务目标声明不匹配。** 现有文件仍声明 identity-only 目标和 identity migration_id；须在核实实际目标后提供 `initialized-identity-bound-financial-target`、财务批次 ID、实际角色与尚未启动应用的真实声明，不能只改旧文件标签。

原始 source export proof 与 raw archive 的绑定已经通过，不再列为缺件。已准入的是固定 Core69 导入再逐段升级的路线；直接替换成当前 Core78 不构成导入批准。Opening-only 与已有 available-wallet06 的范围也不代表全部财产权利完成。审计收据 SHA256 `57a36271d94407eb0815668b5ae16d1f2944f7289fc16704b6e6d65ac2509ae1`；真实目标状态在此次离线审计中未连接核查。

生产迁移的实际查询、连接、写入均为 0。代码和验证继续推进；真实审核／声明、额度对账、writer fencing 与最终升级／失败恢复证据完成后，才能确定生产切流窗口。见 [Extore 接入](extore.md)和[发行架构](release-architecture.md)。

## 历史阶段记录（截至 2026-11-20 11:47 UTC）

以下保留当时的候选、未完成项和证据。这里的“当前”“尚未”等表述均指该阶段的观察时间；其后变化以上方最新状态为准，原失败、原产物身份与历史数字不改写。

- **阶段候选：22c 完整 CI、四包与 Project Z 登记通过；新增客户端进入隔离验收（2026-11-20）**。Extore 分支 [22c5a510](https://github.com/TokenNotIncluded/lmm-api-source/commit/22c5a510a49d49638806e8a8f8315f5c1c52f1fb) 的[原完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44710183213) 已于 11:06 UTC 全部成功，五个 job 覆盖 Web、翻译、真实 Native Extore、Core／PG／支付／Go race／Store／RPC 及聚合检查。四个官方包、四项精确定向回归与 Project Z [06e21c33](https://github.com/LIghtJUNction/project-z/commit/06e21c333e9a05ac092b90ea190d02cfd4e8f60a) 的 30 仓库／51 包登记均保留实际通过证据。发布审阅 [PR #10](https://github.com/TokenNotIncluded/lmm-api-source/pull/10) 当前为 [bbf72df7](https://github.com/TokenNotIncluded/lmm-api-source/commit/bbf72df77a6c144462fc17976a586b4ea4d5b51b)：上一份 fe6 的 Core 与 Native 通过，Identity 的三态存储用例出现 usage 32→33；新提交只补已有异步性能写入的完成前提，三个原完整门槛重新绑定新源码运行。Store／Toolmarket 三个真实 PG 父用例已在 b5ee 全部执行并通过；DSH 五宿主与 Codewhale 九个任务通过，Pi 旧宿主依赖图与 OpenCode Windows 私有目录权限仍有真实失败。六个公共客户端及 append78 已提交独立 [67880af3](https://github.com/TokenNotIncluded/lmm-api-source/commit/67880af3a17e5c0efdd7040fcd62d9e5309f42fe)，目前仅为待捕获精确数据库目录的草稿，不能沿用 22c 的运行资格。默认发行仍为 Core、Go Extensions、CLI；原生 Extore 默认关闭，ABI 边界见 [Extore](extore.md)。

  **已确认并定向修复的 Core 缺陷**：旧 [65 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44659896741) 的真实 PostgreSQL 日志确认 Assistant 取消后的 UNKNOWN 分类存在锁顺序倒置。worker PID 10574 先持请求行锁，在 COMMIT 的延迟触发器中等待会话锁；poll PID 10566 先持会话锁，再等待该请求行，形成互相等待。相关 Core 实现和取消用例在 120、65、73 三版逐字相同；下列既有 PASS 只证明原已测范围，不能排除这个并发缺陷，78 的定向对照已验证修复，原 120／80 基线仍含此缺陷；6102 源码已同步补丁，发行与上机须绑定确实包含修复的新产物及对应完整资格。

  **产品修复与定向验证**：最小修复 [78cb1555](https://github.com/TokenNotIncluded/lmm-api-source/commit/78cb1555197510474f2f5e8f5243c80c698a56c3) 已发布到独立修复分支，将 UNKNOWN 分类改为先取会话锁，再更新请求行并提交，同时向上传递分类与提交错误。[修前负对照](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44683112735) 在冻结 `7b0832d3` 产品上使用同一新增回归，实际仅执行 1 项，exit 101、0 passed／1 failed／0 ignored，精确命中锁顺序倒置断言，PG 日志也记录了 05:49:29.303 UTC 的真实 deadlock。[修后候选](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44683376895) 实际编译冻结 `78cb1555`，完整 `assistant_model` 13／13 在 29.35 秒通过，APP-05 精确目标 1／1 在 6.00 秒通过，均为 exit 0、0 ignored；原取消、资金预留、结算及单次上游调用断言保留。两份 ZIP 实际大小和 API SHA256、控制／产品／旧产品源码绑定、唯一测试 overlay、编译命令、执行前二进制摘要与源码前后稳定均已核对。证据在 `out/lmm-core-targeted-control-draft-78/v2/diagnostic-pair-watch/connector-import/`；运行二进制未收入 ZIP，未声称下载后重哈二进制。此结果是定向 runtime PASS，不替代完整 CI。该分支沿用 Native 证据修复 `1c1582cf`，再接诊断提交 `7b0832d3` 与产品修复；上述定向验收时原 PR #10 尚未移动，后续候选进展见下文。首次控制分支 `f571` 的 `job.env` 引用 `runner.temp` 被 GitHub 解析拒绝，push stub `44682298250` 无 job、dispatch 返回 HTTP 422；该失败保留，不计为产品测试。修正后的独立控制 `5105722d` 通过真实 actionlint 1.7.12，两个实际 run 均为 attempt 1；显式诊断跳过原 protocol，默认完整路径保持不变。

  **APP-05 当前通过，历史失败原因未定**：[7b0832d3](https://github.com/TokenNotIncluded/lmm-api-source/commit/7b0832d37f0a0be0b87cceaf43f7c6a6dfe856bc) 只为 `wrong_system` 失败增加实际 `Result` 输出，原单次调用和精确拒绝 `UPGRADE_DB_SYSTEM_IDENTIFIER_MISMATCH` 断言不变。78 的精确目标已在真实 PG 通过；旧日志没有打印实际返回值，单次新 PASS 不能说明历史失败原因已查明。旧 65 的 Assistant 取消分类与 APP-05 两项真实失败先发生，Rust 步骤以 101 退出；之后 job 达到原 150 分钟上限才被平台取消，annotation 已明确证明该超时。两个预期 `should_panic` 用例通过，不计入这两项失败；完整原日志与输入绑定保留。

  **managed70 当前定向通过，历史失败原因未定**：诊断提交 [c3a38f17](https://github.com/TokenNotIncluded/lmm-api-source/commit/c3a38f17e264b6e88d60ea1709684ef2278bd8a9) 保留原单次调用和拒绝断言，只补实际 `Result`。独立控制 `388da086` 的[精确目标运行](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44687654974) 实际编译该提交的当前 Core lib，在真实 PostgreSQL 执行原 `managed70_owner_upgrade_preserves_all_other_data_acl_and_commit_recovery`，1 passed／0 failed／0 ignored，6.33 秒完成。控制与产品来源、实际编译目录、唯一当前测试执行文件、源码前后及 artifact API 摘要均已核对；未重跑旧 13+1 目标。该结果只证明本次合成升级用例通过，不能解释旧 Identity 的拒绝断言失败，也不替代真实历史数据迁移或完整 CI。

  **Extore 分支的 Core 同步**：6102 以 80 为 parent，仅使用既有 78／c3 的四个完整 blob；GitHub commit、ref 和完整树已回读，保留 8e 私有 UDS 宿主、原 Go／Web 及 feature 独立 `commerce_process` 测试，不混入 main 的 cf 插件路线。覆盖后 Core 运行／lib／协议的 835 个输入及四个外部 include 资源与 c3 一致；c3 定向运行的宽 1,118 项输入仅独立 commerce target 不同。78 的 13+1、c3 的 1 项继续保留其实际来源，这次源同步本身不替代新候选完整资格。旧 120 官方 Core 不含补丁，不能重新标作已修复。[6102 原官方预览构建](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44694301623) 于 07:35:28 UTC 整体 SUCCESS：contracts、Core、Extensions、CLI、Web 五个 job 全部通过，attempt 1，版本 `0.1.27-preview.20261120.6102f997`，未创建 tag、Release 或生产部署。四个实际 ZIP 的 API 身份／大小／摘要、原打包校验器、tar sidecar、组件清单、源码提交、epoch 1795158582 及全部解包文件摘要均通过，root 又独立对比 API 与交付清单。Core 新二进制实际 SHA256 `60d464be991d1adb55676111b656a1cd44320b843608b7441023fa1ff91fa6c7`，四包收据 `out/lmm-distribution-6102-capacity/verification-receipt.json` 的 SHA256 为 `b8fb04f2a6201786209c4d9d038d529d7888fa11677012cd1bad0b67dd60aea3`。该包检查不代表完整 CI；当时按 6102 head 查询仅见该预览 run。该工作流还允许面向 main 的 pull request 与手动 dispatch，但 feature 分支 push 本身不会自动触发完整 CI；后继 22c 的完整运行结果另列。

  **升级拒绝诊断与格式修复**：[c43031dc](https://github.com/TokenNotIncluded/lmm-api-source/commit/c43031dc7b5f0cb399a4faedcd5d69f6368d626a) 仅修改两个测试辅助文件：只在原保留升级器出现非预期拒绝后，按原阶段记录一次有界维护库观察；正常路径不增加查询，既有拒绝保护、成功断言和执行预算保留，SQL 日志已关闭且不输出连接串或 SQL。其[首次原完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44705692145) 的翻译、Web 和真实 Extore 原生集成通过，Rust 格式步骤实际失败。随后用已安装的 Rust 1.100.0／rustfmt 1.11.0 执行原完整 `cargo fmt --all --check`，修前 exit 1；实际 formatter 仅把 helper 中一行拆成两行，修后 exit 0。[ac2fda53](https://github.com/TokenNotIncluded/lmm-api-source/commit/ac2fda53eece85c29c33aaad7cbca0af126d827c) 已发布该唯一格式改动，suffix 文件逐字不变。格式收据 SHA256 为 `7025aea8e996765f715eefa0ec5c36efebfb2bb52ef391f412ef733120276163`；没有据静态／格式检查宣称运行期通过，c430 的真实失败记录保留。 c430 旧 run 后由原 same-ref concurrency 自动取消，未完步骤保留 cancelled／skipped；没有人工取消或重跑。唯一新 [22c 完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44710183213) 已于 11:06:53 UTC 整体 SUCCESS，五个 job 全部通过；原 Rustfmt、Rust／真实 PG、支付 owning runner、Go race、Store／Support、持久事件、真实 RPC 和边界检查均成功。覆盖计数保留原口径：全部 Rust summary 为 2,722 passed／0 failed／78 ignored，其中 lib 为 1,830 passed／33 ignored；普通 Go race 为 2,811 pass／86 skip 测试事件，三个 Toolmarket／Merchant PG 父用例在该普通入口仍全部 SKIP。另一个 Store 专用 PG 入口为 18 个父用例及 9 个子用例，共 27 PASS／0 SKIP。原 run、attempt 1 与 source 22c 绑定不变；终态收据 SHA256 `967207a0682073274a30846a0f4d0b380e400f27adab2456ed933e2ca49922a1`，完整日志和 RPC 原证据另行保留。

  **重启快照的完成前提**：精确源码表明成功的非流式响应在金融结算与 continuation 激活后派生性能记录任务，`performance-v1` 在独立事务中写入；HTTP 完成不保证该事务完成。A5 原用例在成功回复后立即取全库快照，没有等待该既有写入；同文件的其他拒绝用例已处理这类时序。22c 与 fe6 只在 `assert_active` 后、原 baseline 前，复用既有只读 `performance_sample`，保留原 3 秒上限、5 ms 轮询和有效性断言，所有 SIGKILL、同库重启、409、完整快照含 usage、供应商次数及余额断言逐字保留。移除新增两行即可还原原文件；独立审阅与固定版本单文件 rustfmt 检查通过。该改动补齐测试前提，不把生产 HTTP 改成同步统计，也不证明旧失败第五条的实际身份。 [22c 精确回归](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44710812269) 已在真实 PG 18.6 上完成：A5 真实 SIGKILL 后同库重启、三态存储与精确拒绝、原生成功请求精确用量、未知用量保留原 hold 且不重放，四项分别 selected／executed／passed 1、ignored 0。实际编译产品为 22c，控制为 372；1,118 项源码前后相同，原编译命令和测试预算未改。A5 旧／新 PID 为 11631／11636，原 replay 与账本断言完整执行。总收据 SHA256 `9d1bbdfd3e7d6ff0c9a383dd20f1f9d6f3867cf726ec4e5c3424e3e0950d824b`。这是四项定向通过；22c 原完整 CI 随后也独立通过。另一分支 fe6 的完整 Identity 后来发现同文件另一个快照时序问题，见当前 bbf 记录，不能用较早的定向 PASS 覆盖它。

  **22c 官方四包与逐字节比较**：[原预览运行](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44711280545) 的五个 job 全部 SUCCESS，实际 checkout 均为 22c，版本 `0.1.27-preview.20261120.22c5a510`、epoch 1795166180、attempt 1。四个实际 ZIP 的 API 身份／大小／摘要、tar sidecar、原打包校验器及所有解包文件大小／mode／SHA256 均通过，root 已独立比对 API 与收据。Core、Go、CLI 三个可执行文件与 6102 逐字节一致；Web 616 个 dist 文件中 614 个的完整字节与权限相同，入口 JS 只差一处版本字符串，index 只差一处入口文件名，确定性替换后分别与原字节完全一致。包未修改，四包仍不含可选 host／Extore 动态库。本轮只做产物与等价检查，实际短浏览器仍标为 6102。总收据 `lmm-distribution-22c5a510-audit/verification-receipt.json` SHA256 `e9c0033d11aa69d5037976a5d3a6b252c7fbfce1f2f5b31a6193c7366c546fd8`，交付清单 SHA256 `2b6e55c192546d8066e68e543f5c322cce8bc9d1d580ad1b16ad10eeda6b12f9`。

  **6102 新包短浏览器验收**：实际使用上述官方 Core／Go／Web，加上单独绑定的 Extore 8e 原生运行产物；host 编译源仍为 04c234b2、Extore 服务与 `.so` 编译源仍为 81840095，未将它们计入四个官方包。唯一 `browser-preview-6102-dark-01` 按原 S11 frontend 租约与 180 秒总预算执行，实际 58.260 秒、14／14 PASS，587 个响应无 5xx，page_errors 与 observed_failures 为空。真实网页定价、Passkey／PKCE、同用户另一会话 409 拒绝且不消费、原会话刷新回调、1 商品 2 规格导入、暗色切换持久化和 390px 管理／编辑均通过。root 已实际打开桌面、手机顶部、手机完整页和 Extore 重载后截图：Default 徽标完整、手机控件另行，Extore 的两商品内容与操作已加载，补齐旧 80 截图仅为 Loading 外壳的视觉缺口。14 项的原业务断言、5 个 driver 及源码／产物前后稳定；9 个登记 PID/startticks 均退出，原 heavy.lock inode 183521372 保持，08:02:59.489590 UTC 正常释放。收据 `out/lmm-production-acceptance/browser-preview-6102-dark-01/terminal-receipt.json` SHA256 为 `5311ed5c4cf15a74098db78bd527705a86f929c8de018efe5c76ebb6b2b1f396`。原 900 秒到期、重复／重叠同步、断开撤销和五项容量结果继续保留其 120 来源，本轮没有重做或转签。

  **短轮日志保留项**：本地 TLS HTTP handler 读取后续 request line 时出现一份 `BrokenPipeError`；日志最后写入早于后续成功的 Extore 商品 200 响应和 cleanup 快照，原 trace 无逐行时间，不能认定仅由 cleanup 触发，具体关闭原因未确定。后续页面、14 项业务断言与原清场全部完成，原始日志保留。候选绑定的首次预检还发现官方发行 Web 内嵌的是 `inputs.version`，而 CI Web 使用完整 SHA；已据精确发行 workflow 改正 metadata binder，未改产品、source guard、摘要断言或测试预算。

  **正式 CI 与发行构建**：[120 完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44655750741) 的 Web、翻译、microkernel/protocol、真实 Extore 原生集成和最终组件门槛五项全部成功。对应 [官方发行构建](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44654769695) 成功，版本为 `0.1.27-preview.20261120.120e0401`，附件来源与摘要已核验；这里记录构建产物，未将其表述为已发布 Git tag 或生产切换。原 [f229 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44650429692) 的输入清单失败保留，Extore [8e306369](https://github.com/TokenNotIncluded/extore/commit/8e3063690cf5595884b1e639535869fd593e0296) 修复两项夹具摘要后，120 的真实原生门槛已实际 PASS、无 SKIP。

  **Extore 接入流程验收**：固定官方 120 Core、Go 与 Web 的独立完整轮次 16 项通过，538 个响应无 5xx。真实 Passkey 批准后，同用户另一登录会话回调被 409 拒绝且原流程未消费；原会话正常刷新后导入 1 商品、2 规格。两次正常同步与独立重叠查看通过，商品、规格及来源映射不增加；实际等待 Extore access token 的 900 秒自然到期后，再经真实 token 刷新继续读取，连接版本由 1 增至 2。断开后授权已撤销，后续目录请求按合同拒绝；服务、浏览器和租约均已清理。本轮使用真实本机进程、HTTP、独立数据库与合成账号，冻结证据保留于 `out/lmm-production-acceptance/browser-extore-120e0401-02/`。

  **PG 夹具修复与诊断限界**：[efd22485](https://github.com/TokenNotIncluded/lmm-api-source/commit/efd2248548b6744785bf85f5b571b8b19c00de4b) 仅为测试代理的 TCP socket 启用 `TCP_NODELAY` 并补对应回归，保留 Unix socket、COMMIT 响应丢失注入及原断言；没有修改产品连接池、准入、截止时间、两层鉴权或重试策略。此前 off→on→off 诊断观察到授权延迟随传输开关可逆变化，但三阶段 reload 均为 200，不能声称 503 在三阶段复现。setup 的身份存储不可用与旧 503 失败证据保留；未取得 SQLx 原始错误，`PoolTimedOut` 不作为已观测根因。完整浏览器轮次通过官方安装器补齐独立 community 夹具及正确路由，真实导航返回 200。

  **窄屏布局与暗色**：80e02c50 的产品改动只有 `variants-manager.tsx` 排版；390px 下名称与徽标完整，操作另起一行，桌面仍并排，文案与业务行为不变。单文件格式/lint、Web 类型检查、一次真实 Rsbuild 生产构建及首屏体积检查通过。独立短轮沿用官方 120 后端，14 项前段业务与 UI 检查通过，586 个响应无 5xx；已实际查看暗色桌面和 390px 截图，并验证开关可聚焦及真实 Edit→Default→Cancel。该轮未重复 900 秒到期测试，证据保留于 `out/lmm-production-acceptance/browser-variants-80e02c50-dark-01/`。Extore Appearance 的 Dark 切换与重载持久化通过；其内容截图停在 Loading 外壳，完整内容区暗色视觉尚未据此签收。

  **Project Z 成对登记**：[06e21c33](https://github.com/LIghtJUNction/project-z/commit/06e21c333e9a05ac092b90ea190d02cfd4e8f60a) 以 b217 为 parent，将 LMM 固定版本推进到 22c5a510，Extore 仍为 8e306369；只变更 catalog、登记 JSON 和 LMM gitlink。原 `sync`、全图 `check`、`resolve extore lmm-api-source` 及四锁精确恢复后的 `check` 均实际通过，仍为 30 个仓库、51 个包；其余 29 个仓库、51 包元数据和原校验器不变，全部源检出干净，GitHub 与本方 native ref 一致。此前 c430 在额外 offline 环境中因 USTC 缓存缺失而 resolve 失败的原记录保留。本次只对 metadata 使用隔离的 Cargo 1.100 环境、已存在的官方缓存和显式原 Project Z 配置，不修改全局配置、缓存、原校验器或返回结果；不把它描述为原环境重现。四个生成锁字节已保存后精确恢复。原检查收据 `out/project-z-extore-cohort-evidence/update-22c5a510/receipt.json` SHA256 `d643a9b0c8bd455e6c2d31d72072d72335b7c8fb1b06860d64fa373a315cf507`，发布收据 SHA256 `d22108efa57317fbf9e55bd25866862ace1cf87b5316257e55805e2f78309eea`；只覆盖依赖元数据与解析。

  **正式产物容量**：固定 120 产物的五项原门槛已于 04:19:40 UTC 全部 PASS。两档 540 秒均完成 2,100 次模型请求和 480 笔购买到全额退款交易；两档 240 秒均完成 900 次模型请求、180 笔完整退款交易及原 120 条慢连接门槛。以下四档 OOM 均为 0；峰值按各档预算边界计，CPU 与 socket 限流计数分别保留：

  **预算边界**：shared 两档为 Core+Go 共享 1 CPU / 512 MiB，PostgreSQL 在预算外；whole 两档为 Core+Go+PostgreSQL 合计 1 CPU / 1 GiB；Core-only 原 540 秒为 Core 单独 1 CPU / 256 MiB，PostgreSQL、驱动与合成上游在预算外。

  | 场景 | 预算内峰值（bytes） | 请求提交最大滞后（ms） | CPU `nr_throttled` | `sock_throttled` |
  | --- | ---: | ---: | ---: | ---: |
  | Core+Go shared540 | 69,435,392 | 19 | 0 | 5 |
  | Core+Go shared240 | 103,911,424 | 26 | 0 | 337 |
  | Core+Go+PG whole540 | 491,798,528 | 13 | 675 | 0 |
  | Core+Go+PG whole240 | 370,720,768 | 42 | 168 | 0 |

  Core 独立门槛的原运行耗时 586.221 秒；主负载 540,490 ms 内完成 2,100 次请求，拒绝 4,736 次超额请求，2,372 个 UNKNOWN 项在到期、重启后进入 reconcile 分类。主负载的 538 个资源样本记录峰值 48,840,704 bytes、OOM 0、CPU 限流 0、socket 限流最大计数 256；这条 trace 不覆盖后来重启 PID 的分类阶段。五项源码与产物均无漂移，owned 进程、临时 PG 与 cgroup 已清理，外层原资源租约于 04:19:40.726931 UTC 释放且锁 inode 未变。最终收据为 `out/lmm-production-acceptance/capacity120-five-gates-r2/receipt.json`，SHA256 `e09f2df21201e9911574407823efc08b35d8a194338ab6ecae966c7b8cc4fda9`。这些门槛覆盖默认 Core/Go/PG 的本机受控预算，不包含可选 Extore host 的共同部署预算或真实生产负载。

  **发布边界与历史失败**：本迭代链没有生产服务、数据库或入口切换。另一 owner 的 65c7c5c6 路线以 [PR #10](https://github.com/TokenNotIncluded/lmm-api-source/pull/10) 推进。原 `73c01b4d` 候选的 [CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44674251133) 与 [Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44674251552) 均为 FAILURE，实际 checkout 为 merge `4719cba6`。Core 原 Rust 真实数据库、支付 owning runner、Go contract／race 与 durable event 步骤均通过；Store／Support 步骤在独立 pgtest 模块缺少 `purego` 的 go.sum 条目时退出，尚未执行该步业务测试。Native protobuf 步骤的正式扩展镜像构建在 COPY 缺失 Extore 与插件描述符输入时失败，未完成后续 RPC 验收。Identity 的 lib `managed70_owner_upgrade_preserves_all_other_data_acl_and_commit_recovery` 在注入后拒绝断言处失败，原日志未打印实际 `Result`，不能沿用旧 110 项前提失败归因。原 [Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44674251386) 的 amd64、arm64 真实 Alpine 目录检查（step 10）均成功，但两个 job 在 step 11 读取容器 root 创建的 0600 日志／报告时因 EACCES 失败。证据所有权交接修复 `1c1582cf` 的 [Native 原工作流](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44678902022)（attempt 1）已于 05:39:40 UTC 整体 SUCCESS，两架构目录检查、三文件证据交接及上传均成功；该资格只覆盖该提交，原失败仍保留。

  **Store 与 RPC 夹具修复**：[e0befd48](https://github.com/TokenNotIncluded/lmm-api-source/commit/e0befd4863d5114ced42160710705f289c7e4838) 只为独立 pgtest 模块补齐父模块已固定的 `purego` 依赖与两条摘要；已做 Go 模块静态校验，未据此声称 Store 业务测试通过。[d92c9af7](https://github.com/TokenNotIncluded/lmm-api-source/commit/d92c9af762b9eff66d6ac2f070259d61021f17c7) 补齐原 Core protocol 的 Extore 子模块初始化及正式 Docker 输入冻结范围：实际冻结 1,811 个文件，逐条核对 22 个直接 COPY 来源，原 4 项 source-freeze 单测在 umask 022 下通过。首轮 umask 077 的 3 passed／1 failed 保留，原断言未改；文件数量、COPY 来源检查和这 4 项测试均不代表 Docker 或 RPC 运行期 PASS。这条修复沿用该路线的 `cf3469` Extore，不能与上述 80／8e 原生宿主及 ABI 资格混用。

  **d92 静态失败与后续候选**：[d92 Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44688697535) 的两架构 job 因 `scripts/workflow-topology.test.mjs` 仍只预期 `apps/cli` 而失败。[d92 Core](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44688697512) 实际 checkout 为 merge `c4c5c17b`，step 11 原日志确认同一列表断言失败；新候选触发后该 job 自动取消，Rust 编译取消和后续跳过均不记通过。[d92 Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44688697450) 已自然终态 FAILURE（08:01:28 UTC 更新）：实际 checkout 为 `c4c5c17b`，lib 为 1,830 passed／0 failed／33 ignored，managed70 与 `assistant_model` 13／13 均通过；最终只有 `navigation_options_upgrade` target 失败，原保留升级器拒绝 `UPGRADE_DB_OTHER_CLIENT_OR_PREPARED_TRANSACTION`。目标为 1 passed／1 failed，旧日志和严格拒绝保护保留，正在核对连接／事务前提，不将单次失败直接归因于产品或放宽门槛。[6a0a0f4f](https://github.com/TokenNotIncluded/lmm-api-source/commit/6a0a0f4fefe77d0f6d69406ca742ec6619c5c508) 只同步 CLI 与 Extore 的精确预期，原 Node 26.10.0 测试 19／19 通过。该轮 head 为 `6a0a0f4f`、base 为 `65c7c5c6`，实际 checkout 为 merge `be7e13aa`，三个原工作流均为 attempt 1，以下保留其自然终态。

  **6a 原完整 CI 与 Native 通过**：[CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44690182065) 于 08:28:56 UTC 整体 SUCCESS，Core lib 1,830 passed／0 failed／33 ignored、assistant 13／13、导航 2／2；原支付、Go、Store、durable event、RPC 和最终质量门槛均通过。真实 RPC 记录 77 个组件与 289 条命令，重启／替换、撤销、token 轮换、数据库故障重连等原业务检查全部为 true，owned resources 已移除且 cleanup errors 为空。实际 36,849,147 bytes ZIP 摘要及其中 1,810 份源码字节已核验；源清单另外一份隐藏 `.dockerignore` 未进入 ZIP，已单独绑定精确上游 blob，不声称 1,811 份都在包内。Core 完整原日志 SHA256 为 `713f9a18e184fbecf7759a5f9780fbc16ffa0b7f75cd7909f8251d0ba38aa716`。[Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44690182031) 于 07:19:22 UTC 整体 SUCCESS：两架构原源码、构建、安装升级、真实 Alpine Extore 目录、证据交接及上传全部通过；两个完整日志实际 checkout 均为 be7e。巨大 Native ZIP 仅核 API 元数据，未下载验证全包。上述结果属于 main 的 cf 路线，不转签为 feature 的 8e UDS 资格。

  **6a Identity 的独立阻塞**：[Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44690182017) 于 08:56 UTC 更新为 FAILURE，实际 checkout 同为 be7e。lib 为 1,829 passed／1 failed／33 ignored，assistant 13／13 和导航 2／2 均通过；唯一 lib 失败为 `responses_continuity_independent_actual_kill_restart_same_database`。真实 SIGKILL／同库重启后的同 nonce 请求确为 409，随后原完整快照相等断言观察到 usage count 4→5，余额、请求行、continuation 映射和 journal 数均相同。原 2,119,127 bytes job 日志 SHA256 为 `89a3683192bd8e8d403ec319f9321a65166f7a0104c4d26a9b7d7029af0b4b78`；实际 34,059,890 bytes artifact SHA256 为 `550810f4beef8070b06a6a97f7f71c0563e2dda631262da0c95be3cb57df8ce5`，失败源码与 be7e／feature 原 blob 逐字一致。失败后的 a5 记录尚未生成，包内没有第五条事件明细；该故障与 d92 的升级拒绝分别保留。随后 PR #10 推进到 fe6b6ff1，base 65 不变，实际 merge 为 `d4c70abe`；新 [CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709321107)、[Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709320854) 与 [Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709320879) 当时按原配置启动，均为 attempt 1；其后终态见下一段。没有人工取消或重跑旧失败，也未放宽门槛。 新 fe6 [Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709320879) 已于 09:49:55 UTC 整体 SUCCESS；两个实际 job 均 checkout d4c70abe、Extore pin cf3469，原步骤 6–12 全部成功。完整原日志与 Alpine cache warning 保留，巨大 ZIP 仍只核 API 元数据。收据 SHA256 `67181e11fc7b5542b218409db427ec382f7d313d3f8a3d63a3936b1e60b784f0`，不转签为 22c／8e 的整门禁。

  **fe6 完整结果与 bbf 的三态快照修复**：fe6 的 [Core CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709321107) 与 [Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709320879) 实际通过；Core 的 lib 为 1,830 passed／0 failed／33 ignored，assistant 13／13、navigation 2／2，77 组件／289 命令及 RPC 原业务与清理全部通过。独立 [Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44709320854) 实际失败：`responses_continuity_independent_store_tristate_and_exact_refusal` 在原完整快照观察 usage 32→33，其他字段相同；lib 1,829 passed／1 failed／33 ignored，assistant 与 navigation 仍通过。历史新增事件身份未被原日志记录，不能据此认定其具体来源。两份收据 SHA256 分别为 `569fabe4e4f1f5c58f92c9759d25d06d6f765cf1c05142be2f825b53b2f0399e`、`649685c3dbcf3a5ef482b0f7910d232ddb5e99021675170421ba4d316177addd`。[bbf72df7](https://github.com/TokenNotIncluded/lmm-api-source/commit/bbf72df77a6c144462fc17976a586b4ea4d5b51b) 只在六种成功的存储模式组合之后复用原 request-specific `performance_sample`，再进入后续全库拒绝快照；原 3 秒／5 ms 界限、资金、供应商次数、完整快照及拒绝断言逐字保留。独立静态审阅及 Rust1.100 格式实际通过，仍不是运行 PASS。新 [CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44724871920)、[Identity](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44724872007)、[Native](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44724871959) head 为 bbf，PR 登记的 merge 为 95feb5dd；实际 checkout 继续按各原日志核验。截至 11:43 UTC，Native API 已记录两架构 SUCCESS，Core 与 Identity 仍运行；原 attempt 1 与预算不变。旧 fe6 已全部终态后才推进 head，没有人工取消或重发旧运行。

  **全部插件检查与首批修复**：核对范围包括六个 provider 子仓、安装器、11 个内置模块、Extore ABI、Web 安装入口和原兼容工作流。实际 Pi／DSH 旧 CI 分别存在旧宿主自动发现预期、宿主内部依赖导出和 DSH profile 类型不兼容，六份原失败日志及精确源码已冻结，未删旧宿主或改成 SKIP。DSH [19489729](https://github.com/TokenNotIncluded/dsh-lmm-provider/commit/19489729983cbe3c6225816d2594af819ba54b7f) 保留 typed return 和所有 profile 字段／零重试，仅避免 fresh-object 对旧类型的额外字段拒绝，并增补固定 0.2.2-rc.5。其[原矩阵](https://github.com/TokenNotIncluded/dsh-lmm-provider/actions/runs/44714846918) 五行全部 SUCCESS，每行 17／17、0 skipped，真实 source 与打包 tgz 宿主检查均成功；只证明宿主与合成 issuer 范围。Pi 的真实旧宿主命令路径、内部依赖版本集合和陈旧 remote 工作流正在修复。 Codewhale [81a51a42](https://github.com/TokenNotIncluded/codewhale-lmm-provider/commit/81a51a427404ee94abf7b76d414a21505632ff2e) 增加固定 0.10.2 并保留旧版／latest，[原主矩阵](https://github.com/TokenNotIncluded/codewhale-lmm-provider/actions/runs/44721874421) 七个任务及 [Windows 补充](https://github.com/TokenNotIncluded/codewhale-lmm-provider/actions/runs/44721874778) 两个任务实际成功。Pi [9598fc94](https://github.com/TokenNotIncluded/pi-lmm-provider/commit/9598fc94933f5d04bf9382c827b7e228fa6c8a38) 已使用一致的隔离 manifest／原 lock 种子与正常 npm solver，并保留 b782 的 npm12 元数据修复；[原六行](https://github.com/TokenNotIncluded/pi-lmm-provider/actions/runs/44724393909) 中 1.0.4／1.5.2／latest 实际 1.5.2 通过，三个旧宿主仍触发依赖图／undici override 硬门槛，不能称全部兼容。此前 5409／1eb 的真实 extraneous 与重复 chord 实例失败均保留。OpenCode [841b3e5e](https://github.com/TokenNotIncluded/opencode-lmm-auth/commit/841b3e5e62ab75b762d32ccc0ce1eefa403d9656) 增补固定 1.19.6 的九行矩阵，原 macOS 测试在临时目录的祖先安全检查处失败，当时未记录实际 TMPDIR 或祖先路径；[5b21eed3](https://github.com/TokenNotIncluded/opencode-lmm-auth/commit/5b21eed39c085f2d3a7985ac22773f1e047e5309) 仅规范化两个新建测试目录，全部安全负例保留。[后继运行](https://github.com/TokenNotIncluded/opencode-lmm-auth/actions/runs/44725558034) 在 Windows/latest 的 Magpie journal 0700 检查处失败；当前实现还依赖 POSIX UID，且没有 Windows ACL 后端，日志没有记录实际 Windows mode bits 或 UID。其他八行由原 fail-fast 自动取消；其中三行 macOS 和三行 Linux 在取消前实际完成各 52 passed／0 failed／0 skipped；Windows/latest 为 49 passed／3 failed，另外两 Windows 没有完成计数。实际 Windows ACL 支持仍待修复，不跳过安全检查或把整轮取消计为通过。

  **公共客户端接入与数据库 78 草稿**：官方 registry、源码版本与 npm dist-tag 的差异继续保留，不能直接把旧 Web `@latest` 换成尚未发布或仍要求已移除 `remote:control` 的包。完整核对六份 callback／scope／refresh 合同后，[67880af3](https://github.com/TokenNotIncluded/lmm-api-source/commit/67880af3a17e5c0efdd7040fcd62d9e5309f42fe) 在 b5ee 基础上新增 Pi／DSH／Codewhale／OpenCode／VSCode／Zed 的独立 public OAuth 配置、metadata 与 consent，六者默认关闭，各自必须给出有限额度；精确 client／resource／S256 绑定与跨客户端拒绝保持，普通用户不需要开发者等级，但须具备当前资源授权、用户同意和预算。历史金融来源在停止新授权后仍可按原 payer／family 完成结算和释放。持久层只追加 component78 与严格 77→78 owner 升级，不改历史 DDL；OAuth singleton 只允许精确的一字段 catalog CAS，旧行、账本、ACL 与序列继续由原保存检查约束。新增六个真实 HTTP／PG 集成用例和三个普通升级用例；第一个捕获用例明确 ignored，必须显式运行。20 个变化文件、独立源审阅及 16 个 Rust 文件的固定 Rust1.100 格式已核对，编译和 PG78 运行资格仍未获得。目标 JSON 刻意保留 null，使普通初始化／升级拒绝；独立控制 [247dc549](https://github.com/TokenNotIncluded/lmm-api-source/commit/247dc549f171b187ad504494ee8fce56015fb902) 通过 actionlint1.7.12 与语法检查，只用于一次隔离 PG18.6 目录捕获。[首轮捕获控制](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44726411391) 在编译前因新版本解析器误判而失败：保存的 rustc／cargo 均为 1.100.0，但 rustup 状态行出现在版本行之前。没有运行编译或数据库测量，旧 run 与 316,325 bytes 原 ZIP 均保留。[152b3789](https://github.com/TokenNotIncluded/lmm-api-source/commit/152b378976feb3a3d0f6eac3a618455bd5517c47) 仅改两行，以严格唯一的逐行版本匹配接受真实输出，八种错版／缺失／重复／nightly 反例继续拒绝；后继必须用相同 67880af3 产品重新获得独立测量。两个事务必须回滚、输出绑定真实源码，不能把测量写成正式升级 PASS。

  **商店与工具市场三个 PG 父用例实际通过**：原 6a Go JSON 中的三个 SKIP 继续保留。[b5ee7158](https://github.com/TokenNotIncluded/lmm-api-source/commit/b5ee7158157f48b501da526bca96b39c63b50420) 分离 owner 与受限 runtime 的私有 DSN，Merchant 原测试和业务／恢复断言不变。独立控制 095c 的[唯一运行](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44719155709) 已实际执行 Toolmarket 持久化／重启、AI review 丢 COMMIT 确认、MerchantHome HTTP，三个父用例 3 passed／0 failed／0 skipped，原 race 命令全部完成；Go1.27.2／PG18、5,586 项源码前后、最小权限及 42501 拒绝均核对。三个临时数据库、五个角色和私有凭据已清理。原 ZIP SHA256 `b4e1b09085772425a2e2e062e5962cac6c3d0f6d7d297bc927424cb76e12fff2`，独立收据 `d9c2f701473fd00d0b368b0b02d0a178b4ab7be852b13a802cb9fa1393ac3f6a`。仍只覆盖合成 Core adapters／本地 provider／Merchant funds disabled。接入正常 CI 的独立 30 分钟任务和原 095c runner 已形成两文件草稿，原 150 分钟 Core 任务及断言保持；草稿尚未发布或实际执行，不能把一次手动诊断当作永久覆盖或真实资金全链。

  **代码质量检查与人工审阅**：22c 对 6102 的原 `quality-gate.py scan` 实际返回 exit 3／review-required，diff whitespace 通过、无 error，5,586 项源码前后相同。两个信号分别是为旧调用保留的测试 helper `allow(dead_code)`，以及处于 `cfg(test)` 模块的 A5 完成前提；独立源审阅及四项实际定向结果均已保留，原扫描状态没有被改写为绿色，也未增加 waiver／忽略策略。报告 SHA256 `3597bda5cce8181d5113499bfc00c5487555cc066d8291f6dfaa2021e28a76d1`，人工审阅 SHA256 `fb19cafc6c4533035962628fb59fa77d1b17cb4c74ce49a786224dabebd3e5cf`。原扫描未遍历未初始化的 provider 子模块，其质量与兼容检查按各子仓另行完成。

  **真实迁移材料与剩余阻塞**：本次核对找到了原批准 Core69 assembly 与原 ELF，1,004 个实际源码文件逐项匹配；原二进制为 59,276,528 bytes，实际 SHA256 `2315b513839c68d9a794c989fb579f534e75ade45e545075f711c13d2d3d3895`。既有 available-wallet 06 回执记录 real-PG、apply、reconcile 均 exit 0，但 scope 仅 `financial-available-wallet-debt-v1` 且 `all_financial_rights_complete=false`，不覆盖完整经济权利、Key 或整批 opening。原封存 `financial_import.py --check-readiness` 已实际执行：第一份报告 6 项阻塞；核对 producer 和两份 proof 后，仅换回真实原 proof 路径，第二份报告的 `source_file_bindings` 实际 PASS，无需复制数据或修改产品。第二份仍 exit 1、`OFFLINE_CHECK_BLOCKED`，5 个阻塞检查对应 3 类直接问题：本批 source review 缺失、完整 reservations 缺失、传入 identity-target 声明不满足 financial-target 类型及批次 ID；另 2 项是前两项的依赖阻塞。实际 `can_apply=false`，source/target queries 与 target writes 均为 0，未创建经济批准或 SQL；这不是对实时目标库状态的验证。原输入、313 MB raw、archive 与 22 个模块前后摘要稳定，首次 mise 启动拒绝和两份原报告保留。第二份报告 SHA256 `f7acdb750d025a7a895df53e20e3cf00d9c368afa3b4aceecada967d9e5144e0`；独立收据位于 `out/lmm-release-input-fix-evidence/core69-real-import-readonly-review/readiness-execution-20261120-03-original-proof/`。旧 586 隔离暂存与 65 静态准备不代表当前生产资格；同候选产物绑定、真实导入与对账、写入围栏及最终激活仍须闭合。

- **前置完整 CI 与 Core 独立资源测量（已通过；2026-11-20）**：前置提交 c2242c88 的[完整 CI](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44577503481) 已通过翻译、Web、Microkernel/protocol 及最终聚合检查，修复了此前条目所记录的候选后端 CI 未完成状态；该结果只适用于 c2242c88，不直接批准上述新候选部署。
  **实际测量**：冻结的正式 Core 二进制在 1 CPU / 256 MiB 下完成 540 秒场景。正常阶段共 2,100 / 2,100 请求成功，稳态 5 RPS 的 P95 为 90 ms、P99 为 209 ms；过载阶段拒绝 5,760 次超额请求，随后恢复阶段 300 / 300 成功。cgroup 峰值 51,359,744 bytes，无 memory high/max/oom/oom_kill；仍记录到 sock_throttled 226，不能表述为所有事件均为零。2,884 笔刻意制造的 UNKNOWN 资金预留在重启后仍保留并进入核对分类，没有误当作已结算或已释放。
  **范围**：本地受控上游、合成账号和资金；PostgreSQL 与压测驱动不在该 256 MiB 产品预算内。此结果不能证明 Core + Go 联合容量、完整宿主机预算或真实付费模型容量。原始报告及输入绑定保留，独立联合验收继续执行。

- **数据库升级回归修复（源码已推送，定向自测通过；2026-11-20）**：[e312c77a](https://github.com/TokenNotIncluded/lmm-api-source/commit/e312c77a9e5d128d7aa5e09146e66d321cc2bec2) 隔离钱包升级测试继承的 PostgreSQL 连接环境；[827d0e08](https://github.com/TokenNotIncluded/lmm-api-source/commit/827d0e087650757fa11772eb5cefeface7ecab26) 在历史升级测试关闭连接池后等待本夹具连接进程退出，再请求独占升级。
  **自测**：Go 在复现原拒绝的环境条件下，四个顶层测试及三个子场景通过 race 检测；Rust 八条历史升级路径分阶段通过真实 PostgreSQL 自测，涵盖数据、账本、权限、序列及重复执行回执。修改范围格式与差异检查通过，临时数据库已清理；构建执行时限和一次保留执行器路径错误的原记录保留。
  **范围**：本轮修改测试夹具；Core69 历史身份/API Key 导入资格、真实数据与额度对账及独立业务验收继续单列推进。未重跑远程 CI，未发布、部署或切流。见[本轮报告](reports/lmm-upgrade-regression-2026-11-20.md)与[结构化证据](reports/lmm-upgrade-regression-2026-11-20.evidence.json)。**self-tested; independent acceptance not run**。

- **Key quota 验证与 Core77 社区连接历史导入（源码已推送，定向自测通过；2026-11-19）**：[b4414aa5](https://github.com/TokenNotIncluded/lmm-api-source/commit/b4414aa5849f2c1bfc4dd8678c15af1d6d52df48) 修复当前原生 quota 夹具的结构合同发现；[2991afc7](https://github.com/TokenNotIncluded/lmm-api-source/commit/2991afc7eac02f812abed17dbd519f72cf96f6ec) 增加 component77、显式 owner 离线 76→77 升级及精确历史连接导入目标，保留冻结的 55/56 SQL 与 Core69 身份导入审核材料。真实链路还修复 community 单独启用时缺少显示汇率只读权限的问题，补齐启动检查和列权限撤销。
  **定向自测**：Core76 quota 8 项业务加 1 项辅助，共 9/9；Core77 另复验同一条导入→计费业务 1/1，不增加不同用例计数。社区注册表 5、manifest 1、真实升级 1，以及最终历史导入运行链 1，共 8 个不同检查分阶段通过；Go 漏斗用例实际 PASS，Python 计划器 5/5。最终 16 个提交文件与构建输入一致，原始失败、修复和各阶段产物来源分别保留，临时数据库与凭据已清理。
  **范围与剩余工作**：本轮完成当前原生额度验证与保留连接历史的导入/分类/只读漏斗；Core69→当前版本的完整历史身份/Key 导入资格、真实数据与额度对账、完整候选后端 CI 及独立业务验收仍待完成。本轮未发新版本、部署或切流。见[本轮报告](reports/lmm-migration-closure-2026-11-19.md)与[结构化证据](reports/lmm-migration-closure-2026-11-19.evidence.json)。**self-tested; independent acceptance not run**。

- **后端修复与 0.1.21 源码预发布（已发布；2026-11-19）**：已提交 Go 环境隔离 [f8a2868e](https://github.com/TokenNotIncluded/lmm-api-source/commit/f8a2868ebc61fb974eb1e8eb4bd90a0e8b529ee6)、CI PostgreSQL 18 工具 [d40e8a21](https://github.com/TokenNotIncluded/lmm-api-source/commit/d40e8a21116aeabbff1a3a9c45cda58155010000)，以及 Core、夹具与 MK 工作流 [9d50a054](https://github.com/TokenNotIncluded/lmm-api-source/commit/9d50a05465d2680904ebdbb041b3e877a04e1448)。修复可选 managed 模块读取、流式费用补充字段及支付夹具缺表；保持原费用、资金、权限和进程守卫。
  **定向自测**：身份 11、协议 64、钱包／奖励／当前安装器拒绝 3、支付进程 2，共 80 个不同 Rust 用例通过，保留 1 个原有 ignored。Go 5 个顶层测试含 46 个 pass 事件，真实 Ubuntu／PG18 财务夹具 2 项通过。一次 Go 环境纠正和一次 PG 锁容量纠正均保留首轮失败，只复验失败项；owned 数据库已确认清理。
  **后续进展**：当轮的 quota 夹具失败与 55／56 历史连接导入限制已由本页最新 Core77 条目推进，原失败记录保留。Core69 历史身份/Key 导入的完整资格、真实数据对账、完整候选后端 CI 与独立业务验收仍未完成。
  **CI/CD**：[9b9d4234](https://github.com/TokenNotIncluded/lmm-api-source/commit/9b9d4234bd4a926ecc557e076a69615943a68c37)补齐部署前同提交完整 CI 门禁和 runner 镜像 URI 修复，10 项模拟 API、5 项 APT 配置与 19 项拓扑检查通过；真实 API 已确认进行中 CI 会阻止部署。同一发布提交的 Web／翻译通过，官方 Ubuntu archive/security 连接失败导致工具安装退出 100；后端被后续进度提交取消，聚合门禁失败，完整后端 CI 尚未通过。
  **发布**：[v0.1.21](https://github.com/TokenNotIncluded/lmm-api-source/releases/tag/v0.1.21)于 20:34:45（Asia/Taipei）发布，固定源码 [0c8844b0](https://github.com/TokenNotIncluded/lmm-api-source/commit/0c8844b0ea13b67ca25fa6103b712260e8f4011a)。[资格与发布流水线](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44523376980)成功，格式、Clippy 及 289／10／1 项单元检查通过；这 300 项与定向自测有重叠。四份附件、校验和、来源身份及归档内 14 个变更文件已回读验证。本轮未部署或切流。见[本轮报告](reports/lmm-ci-followup-2026-11-19.md)与[结构化证据](reports/lmm-ci-followup-2026-11-19.evidence.json)。

- **主线 CI 回归修复与真实 Core 进程验证（定向通过；2026-11-19）**：已处理上一轮 3 个 Web、4 个 Core 测试失败。Web [04df4c40](https://github.com/TokenNotIncluded/lmm-api-source/commit/04df4c40574cc0999d9c8b009e54fd778c52f8b0) 的[完整 CI job](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44470882275/job/135379764060) 成功：764 个源码测试文件与 6 个脚本测试文件串行通过，格式、版权、lint、类型、生产构建与体积门槛通过。Core 两项合同测试通过后复用，两个测试竞态已修复并精确复验通过；资金、权限与拒绝路径断言保留。CI 有界时限调整为 150 分钟并即时输出失败日志，18 项拓扑检查通过。
  **独立定向验证**：冻结的真实 Core OS 进程、TOML、TCP HTTP/SSE 和独占 PostgreSQL 完成 7 个业务场景，加 1 个支持用例共 8/8 通过；覆盖预算竞争、权限幂等、团队付款与撤权、流式断开、缺失 usage 和重启恢复、结算重试与唯一结算。837 个产品文件、54 个测试文件及 1 项可选路径前后无漂移，数据库已确认清理。40001 为真实 PG 中注入的序列化失败，未证明实际 SSI 并发冲突。
  **边界与证据**：完整后端 CI 尚无最终通过结论；全站、容量及生产切流未完成。合成账号、资金和本地上游不抵扣生产验收。见[本轮报告](reports/lmm-ci-core-process-2026-11-19.md)与[结构化证据](reports/lmm-ci-core-process-2026-11-19.evidence.json)；此前格式、版权及 Clippy 失败保留在[前轮报告](reports/lmm-ci-repair-2026-11-19.md)。

- **请求能力路由实机 HTTP 验证（3/3 通过；2026-11-19）**：通过 ssmcp 在开发机发送真实 TCP/HTTP 请求，使用实际 Core 网关、私有 PostgreSQL 和受限运行角色。次数上限 1 时跳过输出不足的 A、由 B 返回 200；B 真实连接拒绝后转向 C 返回 200，发送编号 1、2 且仅一笔预留与结算；全部不兼容返回 503、零发送和零预扣。重复请求 409，无新增上游调用或扣费。测试 [234643a1](https://github.com/TokenNotIncluded/lmm-api-source/commit/234643a101dcd0bc400b01a0c95cb0ad7ed852bd)。
  **证据与边界**：两个成功场景各结算 10 合成 credits，冻结归零；1,091 项输入摘要前后一致。首次执行 3/3 通过，临时数据库已清理。[实测报告](reports/lmm-request-aware-http-2026-11-19.md)与[结构化证据](reports/lmm-request-aware-http-2026-11-19.evidence.json)已记录。上游为本地测试 HTTP 服务；生产主程序配置启动、真实付费供应商、部署与独立业务验收未执行。

- **渠道配置与 Key 多协议（Web 已部署；2026-11-19）**：当前 Web 为 [9f1bd154](https://github.com/TokenNotIncluded/lmm-api-source/commit/9f1bd154139f154b9e5952c4a9c8922a3e10c50f)。模型选择支持搜索、多选与跨筛选保留选择；合同和路由字段可折叠；渠道提供启停与密钥入口。能力过期或读取失败时保留草稿，暂停写入，刷新后恢复；密钥、批量路由和余额配置采用相同保护。
  **开发自测**：复用源码一致的渠道 52 项回归，重复执行不累加；完整 Web 类型检查与修改范围 lint 通过。1440/390px 的能力过期场景通过；新增 390px 浅深色检查覆盖七个字段、原版本与保存按钮键盘可达性，截图已实际查看，配置写入为零。
  **发布**：[Web run 44363827572](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44363827572) 成功，2026-11-19 02:02:33（Asia/Taipei）记录完成。发布 CI 类型检查、11 项认证回归、18 个 Chromium 恢复场景、构建和制品校验通过；首屏 gzip 499,783 / 786,432 bytes。arch-shadow 与 ubuntu-shadow 均通过健康及实际 HTML／制品匹配检查，本轮只部署 Web。
  **Key 与四协议**：测试提交 [97c50253](https://github.com/TokenNotIncluded/lmm-api-source/commit/97c50253122b8eb305518882d692fab9ae56c2e6) 的隔离 PostgreSQL 定向集合 3/3 通过。管理创建 Key 后四协议均 HTTP 200，每笔结算 49 测试 credits、预留为零；撤销后四入口拒绝，上游和账本无新增。采用真实 TCP、正式管理／模型路由与 PostgreSQL，供应商响应合成；首轮夹具解码失败及修正记录保留。
  **剩余边界**：全库 [CI run 44368105622](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44368105622) 当时记录的 62 个格式失败已由本页 CI 修复条目处理，原始失败记录保留。当前会话无法控制所需已登录 native 浏览器，生产用户链路未验收；本轮没有真实付费模型调用。详见[本轮报告](reports/lmm-channel-key-protocol-2026-11-19.md)和[结构化证据](reports/lmm-channel-key-protocol-2026-11-19.evidence.json)。

- **浏览器会话恢复（首次 Web 发布历史；2026-11-19）**：该修复首次发布的 Web 为 [300d41c2](https://github.com/TokenNotIncluded/lmm-api-source/commit/300d41c2f86acc6471dd30d0e8aeaca5fd9d6d86)，包含 [8abbbdd1](https://github.com/TokenNotIncluded/lmm-api-source/commit/8abbbdd141a221d781279404c154298f7acb95e6) 的原页面回跳修复。受阻的密码提交在改变登录意图前停止；恢复、续期及重新登录保留路径、查询参数和 fragment。失效交接记录和其他标签已恢复后的旧页面通过新的完整文档继续，旧页面内的 Cookie 阻断与待完成写入仍受保护。
  **开发自测**：18 项真实 Chromium 恢复场景、8 项 Cookie API、48 项登录表单、2 项恢复组件及 6 项本地退出合同通过；完整 Web 类型检查与修改范围 lint／格式通过。原始红测与两次浏览器夹具失败均保留在[认证报告](reports/lmm-auth-session-recovery-2026-11-19.md)和[结构化证据](reports/lmm-auth-session-recovery-2026-11-19.evidence.json)。
  **发布与线上回读**：[Web 发布 run 44343809950](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44343809950) 成功，health 与 served HTML／固定产物一致性通过。用户自行登录后，自动化刷新 `/channels` 载入新 Web，未见会话恢复 alert；渠道能力及路由配置读取均为 HTTP 200。本轮未提交用户登录凭据、未写入生产渠道或支付配置；最早一次登录中断的触发仍未确定。渠道能力过期时的草稿保护已随本页 9f1bd154 渠道条目发布；现行 Web 与最新验收边界见该条目。

- **请求能力感知路由（源码已推送，开发自测通过；2026-11-18）**：首个候选的输出上限不足时，继续按既有顺序选择满足请求的候选；不兼容且尚未发送的候选不消耗发送次数。可用备用渠道继续受既有计数、协议及报价合同约束，重试沿用同一笔预留金。代码 [1831c35b](https://github.com/TokenNotIncluded/lmm-api-source/commit/1831c35bf7c047c6af59491ce6a4ce1f06a81c08)。
  **开发自测**：新增 3 项 PostgreSQL 回归全部通过；相关集合 49 项通过，已有 TCP 丢失确认用例在 Unix socket 环境失败，改用私有回环 TCP PostgreSQL 后专项复验 1 项通过，六项共有输入 SHA 未变。最终为 49＋1，共 50 项跨运行通过。首次回归的夹具断言错误及连接环境失败均保留在[本轮报告](reports/lmm-request-aware-routing-2026-11-18.md)与[结构化证据](reports/lmm-request-aware-routing-2026-11-18.evidence.json)。独立业务验收、发布与部署未执行。

- **渠道、AI 路由、用户管理与 EPay 配置（三组件已发布，5004 已导入；2026-11-18）**：Core／Extensions 已部署版本为 [6fa1b905](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e)。GitHub [发布 run 44315659498](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498) 的 Core、Extensions、Web 全流程成功；[部署 job 134848523976](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44315659498/job/134848523976) 于 13:52:42 UTC（21:52:42 Asia/Taipei）完成。Core `check-db` 兼容检查、三组件 health、实际 Core／Extensions 进程可执行文件与 manifest 摘要比对，以及 served HTML 与 Web 产物比对均通过。渠道草稿、故障切换、密钥轮询、目录分页、用户团队／可访问分组合同与 Pancake 独立绑定均保留。
  **上一批发布（历史阶段）**：72392577 的 [run 44305784095](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095)／[job 134813816947](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44305784095/job/134813816947) 已成功，其运行态回执及 75c9d482 首次 Core 摘要各自保留，不作为当前 6fa1b905 的文件摘要。
  **已核验**：渠道／用户的 18 项定向 lib、1 项 vault 持久化、2 项隔离 PostgreSQL 升级测试通过。EPay 最终 16/16、Core 主程序检查 exit 0，13 个 Rust 文件及协议文件共 14 个输入 SHA 未变；Go 定向命令 4 个包均 exit 0（其中 internal/app 的筛选未命中用例），Web 6/6、tsgo／oxlint exit 0。Nginx 真实 loopback 2/2，含 14 个拒绝场景；native-only 管理 include 已上线。独立公网匿名回读：登录页 200、user-access/1 为 401、/0 为 404、Core EPay 配置为 403、Go EPay 投影为 401。
  **5004 实际导入**：固定运维代码 [42c0e49b](https://github.com/TokenNotIncluded/lmm-api-source/commit/42c0e49be8305920051ce37021226e273c41bd35) 的 stage、prepare、activate 三阶段均成功；最终安全回执为 `activated`、`recovery_required=false`、`helper_exit_code=0`、`release_verified=true`。Core 已切换到私有持久 base，Core／Extensions 就绪检查通过。5004 保持禁用；已有资金开关、Pancake 配置及 Go 5002 原有 `subscription_enabled=true` 均保留。[完整阶段回执](reports/native-channel-routing-20261118.md#epay-5004-实际导入回执)记录实际路径、权限、plan 摘要和运行链接；一次性入口已在 [56f5f7ab](https://github.com/TokenNotIncluded/lmm-api-source/commit/56f5f7ab825d5828e5125c97b199f4facfa62a36) 撤下，固定 Git 历史与安全回执保留。
  **渠道超时修复（源码与发布均完成）**：[6fa1b905](https://github.com/TokenNotIncluded/lmm-api-source/commit/6fa1b9054c1a03898e082aacf28e228880c3004e) 已进入 main 并完成三组件发布。Core 返回实际 `model_limits.total_timeout_ms`，对低于运行下限的启用 upstream 目标返回专用 409 `CHANNEL_TARGET_TIMEOUT_TOO_SHORT` 及 `required_min_timeout_ms`；Web 新目标使用 `max(30000, actual)`，保留既有值，执行仍使用全局 deadline。Core 真实 HTTP／PostgreSQL 3/3，编译 6m19s、测试 4.09s，5 个输入 SHA 未变；Web 5 文件 77/77，完整 tsgo、修改范围 lint／格式均 exit 0，4 个新 key 的七语言内容完整。首次 Web lint 的两条 curly 错误已修复，原失败日志保留。
  **此前浏览器回读（会话恢复批次）**：当时用户已自行登录。会话恢复修复发布后，自动化刷新 [渠道页](https://native.api.lmm.best/channels)，新 Web 无会话恢复 alert，渠道能力及路由配置读取均为 HTTP 200。现行 Web 见本页渠道配置与 Key 多协议条目，认证历史证据见会话恢复条目；本轮未再次观测该已登录会话，渠道、用户或支付配置的完整业务验收仍待继续。本轮用户分组只读。
  **剩余事项**：native 数据库已完成 74→75→76 升级，服务器备份及数据库备份的隔离恢复校验完成；开发机第二份完整数据库／运行时备份复制仍因 SSH 握手阻塞未完成。继续渠道超时页面读回及既定业务验收；本次浏览器未执行生产配置写入，EPay 返回 URL 保存／读回／恢复测试尚未执行。6fa1b905 已完成 CI 运行态验收；开发机 SSH 独立回读仍受阻，属于补充复核。失败、中断、修复及证据入口见[本轮报告](reports/native-channel-routing-20261118.md)；以下条目保留各阶段自己的版本与验收边界。

- **分组请求限额逐行编辑（源码已推送，开发自测通过；2026-11-18）**：管理页支持逐行添加、修改、移除分组限额，保留精确 Unicode 组名，保存前拦截重复名称与非法次数。版本号、会话隔离、冲突草稿和显式重新读取沿用既有合同，七语言同步完成。代码 [c1890bc6](https://github.com/TokenNotIncluded/lmm-api-source/commit/c1890bc673c3751c992fb536a2180959b756a004)。
  **开发自测**：7 项定向测试、修改范围格式与 lint、完整 Web 类型检查、生产构建与包体检查通过；真实 Chromium 交互及 1440/390/320px 明暗、中/英/法四种布局通过，截图已实际检查。浏览器使用本地合成接口。独立业务验收未执行，未部署。[使用说明](request-limits.md#管理请求限额)；[本轮记录](reports/lmm-request-limits-2026-11-18.md)。

- **渠道批量路由与模型估算器（源码已推送，开发自测通过；2026-11-18）**：渠道支持批量修改优先级、权重、超时，留空保留原值，草稿绑定选择与版本，冲突刷新后不自动重放；纯输入端点模型隐藏不适用的输出、缓存及聊天预设。代码 [be27b613](https://github.com/TokenNotIncluded/lmm-api-source/commit/be27b6133ac6d0ea5ae684631be6660da35dbcbc)、[028eb4e3](https://github.com/TokenNotIncluded/lmm-api-source/commit/028eb4e3a885438fab0319cf2fa265be6998a58a)，预览夹具修复 [9aba1b87](https://github.com/TokenNotIncluded/lmm-api-source/commit/9aba1b872725c24d78a0c3d87d17f306d4f1a23d)。
  **开发自测**：渠道 60 项、估算器 11 项通过；完整类型检查、修改文件格式与 lint、生产构建及包体检查通过，首屏 gzip 499,589 B。Chromium 的 1440/390 px、明暗及中/法/俄六场景 6/6，最终截图已实际检查；使用本地合成数据，未写入真实后端。首次预览错误横幅和一次类型检查超时均保留在[迭代报告](reports/lmm-channel-routing-2026-11-18.md)及[结构化证据](reports/lmm-channel-routing-2026-11-18.evidence.json)。未部署，独立业务验收未执行。[交付附件](https://drive.google.com/drive/folders/1Ixnf5t8di1HmVyWJUu6F9L2bQ-jP2yL3)。

- **定价导航与未知计数修复（源码已推送，开发自测通过；2026-11-18）**：代码 [5080810c](https://github.com/TokenNotIncluded/lmm-api-source/commit/5080810c72e47535c4bb0fafe63f37199a5942cd) 已进入 main。路由改读现行 `/api/site/navigation`，配置要求登录时保留原地址跳转登录，导航读取失败继续关闭入口。目录尚未取得有效数据时，价格页隐藏模型数量、筛选器、厂商空态及价格基准说明；模型面板同步隐藏未知计数，重试成功后显示真实空目录。
  **开发自测**：导航与认证 8 项、价格状态与会话隔离 7 项通过，修改文件格式与 lint 通过。共享工作区的类型检查被既有渠道文件合并标记阻塞；从 a83dbf27 固定归档覆盖本轮六文件后，完整类型检查、生产构建及包体检查全部 exit 0，首屏 gzip 499,663 B。实际候选静态产物在 Chromium 中完成 1440px/390px 明暗错误与重试场景及登录回跳，共 5/5；接口使用合成响应，外部网络封锁。独立业务验收未执行，生产仍为 Web 3a9986d9。[结构化证据](reports/lmm-pricing-navigation-2026-11-18.evidence.json)；[交付附件](https://drive.google.com/drive/folders/1U9kWpknGqHWwBnDWaXmzFKb2qtnGdh43)。

- **新架构前端生产化（Web 已部署，浏览器 5/6；2026-11-18）**：最终 Web 固定为 [3a9986d9](https://github.com/TokenNotIncluded/lmm-api-source/commit/3a9986d92ce8c464e4ef3fa67ee521f6e0fde5c9)，06:18（UTC+8）部署至 https://native.api.lmm.best，公开 api.lmm.best 继续运行旧 Go 0.2.118。补齐共享模型/厂商空态，整合导航、最近访问和快捷入口，可选 WebMCP 按浏览器能力载入。四处过时或不完整的测试夹具/断言已修正，生产校验与权限逻辑未为测试而改变。
  **最终门禁**：760 个源测试文件 + 6 个脚本测试文件全部通过，exit 0、860.875 秒；完整类型检查、生产构建、标准 Web 包校验均通过。最终生产 dist 对同一新架构固定基线的七个首屏 JS/CSS gzip 从 609,043 降至 500,178 B（−17.8748%）；三场景强制 GC 后 JS 堆中位数下降 0.9363%–4.7377%，18/18 导航有效。这些比例不表示相对旧 Go 的性能收益。
  **实际部署与独立回读**：625 个文件逐项核验，首页与全部 10 个引用资源一致；两代保留静态文件实盘核对及六项 HTTPS 抽样通过。独立回读 20/20，Core/Extensions 的进程与二进制、Nginx 的进程和配置未变，旧 Web 槽位和本次上传目录已清理。实际组件仍为 Core c67892ba、Extensions dc94d4c2、Web 3a9986d9，包保持 preview=true。
  **浏览器与剩余阻塞**：真实匿名桌面/390px 明暗登录页及工具市场回跳通过，Native 为 5/6、整体 exit 1；旧 Go 登录页两场 2/2。本轮回读现行 /api/site/navigation（revision=2）确认 pricing.requireAuth=true，匿名 GET /core/v1/pricing/public 返回 403 pricing_forbidden 符合当前配置。此前关于允许匿名的判断来自旧路由读取缺少导航字段的 /api/status 后采用默认值。线上 Web 3a 的路由与错误零计数仍待部署本轮修复；Extensions 导航呈现和 Core 定价授权维持各自边界，后续策略调整需明确 Core 迁移方案。生产登录态概览/钱包/Root 保存、真实资金、外部模型和后端迁移核账仍未验收；赏金、公共中转 config 仍 503。已删除的激活付费、平台等级、远控/Pi、FastPay、任务日志和客户端配置页面保持移除。
  完整测试失败与修复、性能方法、部署与回退边界、实际新旧对比及限制见[最终部署报告](reports/lmm-frontend-production-2026-11-18.md)和[证据摘要](reports/lmm-frontend-production-2026-11-18.evidence.json)。文档后续提交不改变本次上线的 3a 身份。

- **控制台页面补修（历史候选记录，当前 Web 见本页首项）**：商品与消息页补齐七语言实际缺失的文案，并将额度显示接入翻译；本轮新增 36 个 key，合并时保留并行任务的既有译文。任务日志与客户端配置的导航、设置和工具入口已清理，旧地址分别回到使用日志与开发者入口，见 [9ca5258](https://github.com/TokenNotIncluded/lmm-api-source/commit/9ca52583f68f669a8ccd7973a87175db5cd68c27)。原界面将后端 `model-specific` 误显示为“自动倍率”，现显示“按模型定价”，真实数值倍率单独显示；计价规则未改动。
  [6eec91b](https://github.com/TokenNotIncluded/lmm-api-source/commit/6eec91b9d6d84dc8b79c61b781abbebcee0b98cb) 增加匿名 `/health/ready` 检查，将 API、数据库、模型路由就绪状态与历史请求性能分开展示；无采样时解释数据来源，公开检查与私有会话缓存隔离。钱包余额和转账记录分别处理读取结果，记录失败不会清空已取得余额；只读请求恢复会话续期，资金写入仍不自动重放。渠道列表保留直接编辑/测试入口，次要操作收进详情，协议改为可读选项，密钥输入前置；[b2c64dc](https://github.com/TokenNotIncluded/lmm-api-source/commit/b2c64dcd43827211eef2b6c7d187c9ad1339cb59) 根据实际截图修正创建表单背景，使输入区清楚可见。
  发布候选固定为 [5440888](https://github.com/TokenNotIncluded/lmm-api-source/commit/5440888b4bd5593b1f92a21463d1fd3ea07cb59e)，标签 `web-ui-console-20261117-5440888` 已创建并读回；候选已纳入 main 历史，配方仓库 [872846f](https://github.com/TokenNotIncluded/lmm-api/commit/872846fe7a4de0ebdf51fd450daa4792ae1bf51b) 固定相同 gitlink。后续并行系统设置改动保留在 main，不包含在本发布候选。
  本轮 175 项定向回归通过（包含恢复后重跑的 107 项及发布前 11 项认证回归，重复执行不重复计数）；渠道/状态共 12 个桌面与手机浏览器场景通过，最终截图已实际检查，另有 16 个真实 Chromium 认证恢复场景通过。固定候选的完整 Web 类型检查、全 Web 格式检查、生产构建、包体检查和标准 Web 包验证均通过；首屏 gzip 为 589,907 bytes，门限 786,432 bytes。早先本地类型检查退出 137、浏览器 socket 限制及 CI 两次格式失败保留为历史结果，不能写成那些尝试通过。
  发布包 `lmm-web-0.0.0-preview.console-20261117-any.tar.gz` 为 19,374,682 bytes，SHA256 `bef6aaa90deb79f0a9c9c9d25dc82725f433470d07c16f5aa05d7428acfae13f`；构建 index SHA256 `e5b205ea3973d562b7106441cffdc70cbb1074b62c05eb9445ddd265cca25f5c`。开发机证据位于独立目录 `.lmm-console-review/` 的 `out/distributions/` 与 `scratch/`；未覆盖原开发工作树，临时浏览器和 5177/5178 服务均已结束。
  **该候选当时未部署（历史记录）**：已读取 GitHub shadow 保护规则，仅允许 main、shadow-v*、v* 和既有 Ant 精确标签。当时发布该固定候选需要用户确认新增上述精确标签授权，再按 `components=web, targets=arch-shadow` 运行现有部署流程。没有修改保护规则、服务器白名单或线上配置。当时预览域名从本环境与开发机均返回 403，真实健康代理与钱包后端尚未线上读回；浏览器使用合成响应，不替代真实转账或独立业务验收。

- **系统设置、定价、Hero SMS 与签到配置（native 已部署，登录态业务验收待续，2026-11-17 UTC）**：恢复连接后补齐 Hero Core 的私有 settings/save-policy/test 实现，Root 会话及完整导入权限重新验证、精确倍率、版本比较和原子持久化已接入 Go/Web；购买能力明确关闭。签到增加独立 GET/PUT 管理接口、版本冲突及未知写入恢复，生产以正式安装与发布命令建立专库和 revision 1 策略，运行账户只获受限保存函数权限；没有启用 Core 奖励资金或执行领取。独立配置不再受旧 options 只读状态整体禁用，定价的精确编辑、草稿、差异预览和确认发布、Pancake 设置流程、公告草稿保护及 OAuth 恢复均在固定 Web 中；无消费者的自动注册／邀请奖励字段改为真实现有入口。
  **该轮发布版本（Web 已由本页首项接续）**：Core [c67892ba](https://github.com/TokenNotIncluded/lmm-api-source/commit/c67892ba5be613bf2ef6de51a4b841043f9b881e) 保持现网 74 组件合同，Extensions [dc94d4c2](https://github.com/TokenNotIncluded/lmm-api-source/commit/dc94d4c2562db52537e3abbced030f6da32ed78f)，Web [561faf51](https://github.com/TokenNotIncluded/lmm-api-source/commit/561faf51a74a4000ebb2c7d812da4230eba3a213) 于 21:01:47 UTC 切换。目标为 https://native.api.lmm.best；公开 api.lmm.best 仍运行旧 Go 服务。三个固定组件不等于随后并行前进的最新 main。
  最终 Web 完整类型、定价 7 项、生产构建和标准包验证通过，首屏 gzip 589,234 / 786,432 bytes；真实编译页面的 24 个受控 Chromium 场景通过，生成 56 张截图，桌面／手机菜单、明暗主题与关键页面实际看图。610 项生产文件清单全部匹配；独立直接 HTTPS 回读首页及全部 7 个初始脚本／样式摘要一致，旧版本独有入口、延迟加载脚本和 CSS 仍为 200，缺失静态文件为 404。Hero 单元 5/5、真实 PG 1/1、Core74/76 各与 Go 的双进程验收 17/17、owner 权限边界 46/46 通过。基座 2488 的完整 Web 命令执行 783 文件，其中 782 退出 0，唯一失败为 Web-only 归档缺少根 contracts 夹具；完整同 SHA 归档仅补测该文件 1/1 通过，原 aggregate exit 1 和 CI 被并行 main 取消的事实保留。
  新增四条 native 精确设置路由，经实际 HTTPS 和上游日志核对到 Extensions；匿名访问按预期拒绝。Core／Extensions 实际运行文件、签到策略与权限已独立回查，供应商只读连接返回有效响应，没有购买或资金操作。旧静态文件独立保留；Web 首次切换因 native alias 未使用内部端口的 fallback 而自动恢复，修正 native 局部文件选择后重新发布并独立核验。临时上传包已清理，保留一套旧组件槽与完整恢复归档。
  **仍未完成**：用户真实登录浏览器控制桥仍不可用，未复现公告原登录态 503，也未在生产 Root 页面完成保存／连接测试；Hero 购号与邮箱购买、签到奖励领取／资金授权／历史导入、自动注册／邀请奖励、真实支付交易与公开域名切流均未完成。匿名 401、只读供应商连接和受控浏览器场景不能替代这些业务验收。完整版本、测试边界、失败与回退证据见[设置部署报告](settings-deployment-20261117.md)，开发机证据目录为 ~/.local/state/lmm-api/settings-closure-{core,extensions,web,rollback}/。

- **渠道配置与 Rust 依赖更新（部分源码已推送，候选验证未完成，2026-11-18）**：[1362b3ac](https://github.com/TokenNotIncluded/lmm-api-source/commit/1362b3ac2c741c83cf955f5880d62bf28ab7ce4d) 更新依赖并适配可失败的 JWT 密钥构造；[112aedb4](https://github.com/TokenNotIncluded/lmm-api-source/commit/112aedb4442d496cf60c10eab0e2af48ab81f6f1) 增加版本绑定的原子批量渠道配置；[0de5ca6d](https://github.com/TokenNotIncluded/lmm-api-source/commit/0de5ca6d5938c8bc4e8d36e81df3dee62c9cd503) 对混合更新中的启用、标签和路由参数分别检查所需权限，全部授权通过后才修改。三项提交均已在 main，依赖版本见[Rust 依赖审查](rust-dependencies.md)。
  固定 74 组件候选的 Core release 构建与标准发布包验证通过；隔离 PostgreSQL 的批量配置 3 项通过。原基座 vault 测试 3 项中 2 项通过，1 项旧传输夹具因内存路由与数据库版本不一致返回 503，失败记录保留。已核对主线规范恢复替代用例保留关键断言，但该用例及 JWT、PSL、sysinfo、relay/zconv 定向库回归尚未启动，不能记为通过。
  前端卡片/表格和分区编辑器候选已有阶段性自测及桌面、窄屏检查；最终输入边界与密钥占位提示修正、26 文件 UI 提交及最终 Web 发布包仍未完成。当前项目开发机的 ssmcp 连接不可用，无法续跑或读回余下收据。本轮没有激活 Core/Web，没有完成线上验证或独立业务验收；已完成的构建与局部测试不代表渠道功能已上线。候选仍固定为现网兼容的 74 组件基座及本轮补丁，不包含 main 并行新增的第 75/76 组件。开发机证据入口：`out/channel-management-20261117/`；恢复连接后先核实既有任务结果与当前部署基线，再继续验证和发布。

- **原生地域与用户 IP 白名单（Core 专项 19 项通过，尚未部署，2026-11-18）**：Rust Core 新增公开入口策略、进程内 MMDB 地域查询及可信 edge 元数据传递；用户 IPv4/IPv6/CIDR 白名单接入 Session、密码/MFA/OAuth 签发、浏览器刷新/恢复和模型/Key 读取。用户和 Key 规则取交，保存非空用户名单须包含本次服务端来源 IP；旧账户空设置按未配置处理。平台 `IPAccessRoutingRules` 接入保存、批量比较保存与审计，固定键函数保持 runtime 无 options 表直接写权限。配置与接口见[地域访问控制](network-access.md)。
  界面已推送 [44728f3](https://github.com/TokenNotIncluded/lmm-api-source/commit/44728f37d7ec3107a703b5a1e5f01e77c5253e0e)：真实设置页按可写 capability 挂载地域编辑器，个人卡片按用户/会话绑定请求，七语言通过 helper 同步；35 项定向测试、修改范围严格类型检查、lint 和局部真实组件构建通过。全项目 Web 类型检查两次被环境 SIGKILL/137 终止，无 TS 诊断；未完成浏览器截图，不能据此宣称视觉检查通过。
  新增第 76 组件及标准 `core-network-access-v1`。隔离 PostgreSQL 18.6 下，75 前置 manifest/catalog 均匹配冻结合同；追加 76 与全新 76 的实测 catalog 一致，目标 manifest `2c14283c2d35580f3fe314b7de6a88e3aec3524e74157310bf5022415367ff8e`、catalog `bed3e7e3ad2810bda942473b660e4348294dab31e8fdc72e52cb5693507158db`。这是此前原始 SQL 与正式 catalog query 的测量，当时未替代 Rust 原生初始化或升级回执验证。 新 writer 的 10 项受限 runtime SQL 检查通过，覆盖实际插入/修订、直接表写拒绝、空/超限参数拒绝以及显式和异常事务回滚；当时 HTTP 审计路径尚未运行，已在下述本轮专项补验。
  此前 Core 的 lib-test 和较小 production-lib 集成目标编译均在 Core 阶段收到 SIGKILL；当时未取得可执行测试产物，原生 HTTP、用户白名单和升级/replay 验证留待本轮补齐。历史失败及原 SQL 证据保留在此前交付包和 `out/network-access-validation/`。
  **2026-11-18 接续完成**：[e1b461a7](https://github.com/TokenNotIncluded/lmm-api-source/commit/e1b461a7b710ae10e56e751b67a61db57512885a) 增加 `scripts/test-network-access.sh` 和统一说明，以私有 Unix socket PostgreSQL、隔离连接变量、单任务构建及串行测试复现专项。首次 17:05（UTC+8）运行因客户端重启中断，退出码未知；恢复后取得 13 通过、1 失败，数据库日志确认 Key 测试夹具缺少 `core_abuse.settings`。已用 [ef8d0130](https://github.com/TokenNotIncluded/lmm-api-source/commit/ef8d01304ecff6b7005c05222e20e37d5a20384c) 补齐该测试的真实 abuse schema 初始化，原业务断言保留。
  固定 `ef8d0130` 于 17:40:11–17:46:51（UTC+8）完成修正后专项：**14 个 lib 用例、2 个管理配置接口用例、3 个用户网络策略接口用例全部通过，共 19 项、0 失败，退出码 0**。覆盖地域/来源校验、用户与 Key 规则交集、登录与 Cookie 恢复、设置保存与账户隔离、受限运行权限、比较保存冲突及审计失败回滚，以及 75→76 数据/权限保留与升级回执重放。Rust 格式、Bash 语法及文档/差异检查通过。
  本轮临时数据库已正常停机，成功运行临时目录及修正后的失败现场已清理；原失败 PostgreSQL 日志和三次运行记录保留在父仓库 `out/lmm-network-access-20261118/`。最终收据为 `network-tests-3.json`，完整日志为同名 `.log`。这是开发自测，独立验收、生产升级、部署和新限制启用均未执行。[本轮记录](reports/lmm-network-access-2026-11-18.md)；[Google 云盘副本](https://drive.google.com/file/d/1XJTcvp7ybWXCL6f7A5osFYgXlByVrCGY/view?usp=drivesdk)。

- **Native 接口 503 与申诉请求修复（前端、Scripts、AI/RSS 已部署；赏金与公共中转仍未完成，2026-11-18）**：[15c26f59](https://github.com/TokenNotIncluded/lmm-api-source/commit/15c26f59cfb4bb5b676966b8aa165708042a0c71) 对明确的 `core_migration_incomplete` 停止自动重试，普通暂时性 503 保留既有重试；赏金按实际标签加载，首屏赏金请求路径由 7 项减为列表与配置 2 项，保留手动重试、费用和待办依赖。[64bd8f42](https://github.com/TokenNotIncluded/lmm-api-source/commit/64bd8f423d821cd79a5bca87bf8471c4ac0adfe1) 仅在已验证的账户禁用响应后读取申诉上下文，普通登录和手动密码申诉不再无条件请求；保留 OAuth 跳转、过期处理和服务端证明校验。[04fef6f1](https://github.com/TokenNotIncluded/lmm-api-source/commit/04fef6f15b8214b56494ad2af8f3e2c8076035af) 修正导航键及退休账户字段的类型不匹配。
  此前 Web 从固定提交 `04fef6f1` 部署为 `0.0.0-ui.20261118.04fef6f`。5511 个受控文件与该提交逐一一致，断线前完成的修正后类型检查已读回 exit 0；生产构建、包体及归档校验通过。原行为自测 21 项通过，发布所需 Cookie 回归另 11 项通过；真实 Chromium 在受控服务上完成 16 个 Cookie 恢复场景，属于开发自测，未作独立业务验收。实际 HTTPS 登录页和全部首屏 JS/CSS 与产物哈希一致，Core 状态 200。包 SHA256 为 `083913ed5092cdc24c2e031bd5aca96741031a0546c738339ebfd7a5c2f75dfb`。
  AI/RSS 两条精确路由已接入 Extensions。Scripts 原仓库快照为空，按既有静态清单机制接入旧站冻结的 58 个真实脚本（87,462 B）；只重启 Extensions，native 专用上游映射通过 Nginx 检查并 reload。许可来源的最终 HTTPS 读回：scripts 200/58 项、AI 200、匿名 RSS 401 `OPERATIONS_LOGIN_REQUIRED`、Core status 200；脚本两种 raw 路径的抽样字节哈希一致，旧站脚本响应不变。保留 operator ACL，开发机外部访问仍为既有 403；登录态 RSS、线上赏金请求计数及真实申诉流程尚未验收。
  赏金：当前安装 Core 在候选配置下只读 `check-db` 通过，已有专用 token/socket/schema 可复用；当前费用策略行不存在，Extensions 缺少 contract SELECT 和 records SELECT/INSERT/UPDATE，contract 版本因 42501 尚未验证。历史已迁移报告指向另一数据库，不能作为当前策略依据；尚未改费用、权限或启用赏金配置。公共中转：当前 Core/Extensions 没有对应模块和私有 authority；旧实现直接改旧余额/渠道，不能通过转发或复制旧模块接回。两个 config 接口仍为 503，不能宣称全站恢复。
  配置和旧前端完整恢复材料已在开发机 `~/Backups/servers/archdmit/` 校验，实际回退未演练；服务器已清理本轮旧 Web 槽位、上传包及配置临时文件，只保留当前内容。原失败和修正证据保留：首次类型检查失败、诊断字段覆盖误判，以及未派发的 ssmcp 繁忙/审计写入失败。Core 未重启，未执行脚本、生产账户测试、付费调用、资金操作或数据库迁移/授权/策略写入。

- **Ant 界面、加载/错误页与设置修复（已部署，2026-11-17）**：[b7c7693](https://github.com/TokenNotIncluded/lmm-api-source/commit/b7c7693b3d165d23a50fb52f3700e36388c9aa20) 简化首页抽象粒子；[bf61dc3](https://github.com/TokenNotIncluded/lmm-api-source/commit/bf61dc3cef0e4e913f79ccec34d59288e81f4491) 将默认主题与外观名称改为 Ant，兼容旧偏好迁移，统一完整视口加载页与 401/403/404/500/503 错误页。设置页修复完整后端能力误判、刷新覆盖草稿、保存后旧缓存回滚及保存期间新增编辑丢失，受限后端按分区展示实际可写控件和只读值；未新增所有旧配置的服务端写入能力。
  本轮 91 项定向测试、Web 类型、生产构建与包体检查通过。构建产物在 17 种首页/加载/错误页面配置下通过浏览器检查；调试入口的 6 种设置页面配置及桌面/手机主题持久化、恢复默认检查通过。浏览器使用受控 API 数据，管理员配置真实写入和数据库持久化未验收。陀螺仪几何点数由 23,468 降至 9,888，未据此声称实机帧率或功耗改善。
  用户批准精确标签后，为 shadow 新增 `web-ui-ant-20261117-bf61dc3` 标签规则，原 main、shadow-v*、v* 规则保留。[运行 44092321096 第二次尝试](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44092321096/attempts/2)复用已通过类型、认证回归、真实浏览器恢复、生产构建、包体及制品验证的相同产物，仅重跑部署，21:51（UTC+8）完成 arch-shadow Web 更新。服务器 failed/unreachable/rescued 均为 0，后端重启步骤跳过。21:53:01 线上首页、登录、404/500 路由入口及 7 个首屏 JS/CSS 均为 200；HTML 与全部受检资源 SHA256 精确匹配该制品，`/api/status` 为 200 和有效 JSON，主 JS 确认完整 bf61 提交号。公网与制品 index SHA256 均为 `4753554ed6c151a9fd9f07d082e493744a64c3ded02af5f957358df25fbd4df3`。未更新 Core/Extensions、访问白名单或生产管理员配置。
  首次标签拒绝与另一次 main/a4f633e 的并行用户契约类型失败保留为历史记录；本次运行 Web 固定为 bf61，不包含其后 main 的迁移。开发机证据：`packages/out/ui-refresh-review/evidence/production/report.json`、`evidence/settings/report.json`、`evidence/live-bf61dc3.json`。本轮属于开发自测、实际部署和线上文件回读，独立业务验收及生产配置保存未执行。

- **Native 入口 403 与控制台 Web 更新（已部署，2026-11-17）**：Nginx 的 `access forbidden by rule` 与当前访问请求对应，操作者网络出口变化后未命中预览站 IP 白名单。已在 native 域名单独补充当前操作者允许规则，`nginx -t` 与热重载均退出 0。共享预览访问限制继续生效，更换出口仍需同步白名单。
  本轮完成充值到账/退款确认后的只读余额刷新（[ec412878](https://github.com/TokenNotIncluded/lmm-api-source/commit/ec41287861f560bb5244d962f15cc24f686835bf)）、AI 入口与侧栏工具简化（[4ab81a9b](https://github.com/TokenNotIncluded/lmm-api-source/commit/4ab81a9b35e906198046d9e7a4a8ff6a03cc60a0)）、账号密码优先及条款勾选前置（[f7c41b8d](https://github.com/TokenNotIncluded/lmm-api-source/commit/f7c41b8da282d2121a1111fd826fb4b101f41e90)），并精简替代登录区域的重复分隔提示。余额仅在服务端确认到账或完成退款后刷新，同一订单同一终态去重，保留账户/会话范围检查与原资金请求恢复语义。
  固定 Web 源码 `5e918ee95d95d770c360eed4b4a8554941f1bfe8` 经[部署 44067641519](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/44067641519)发布至 `arch-shadow`。2026-11-17 18:49:22（UTC+8）回读首页、登录、钱包、状态及 7 个首屏资源均为 200，未登录管理员接口为 JSON 401，旧站首页为 200；公网与服务器 index SHA256 均为 `32fa19e092cd5efdde214bdd99425dd3feb5d36bd8d00e294646b0d3184a6e18`，Core PID 前后均为 269501、ready 为 200。
  钱包 19 项、导航 25 项、登录 48 项定向自测通过；最终源码的 47 项登录表单回归再次通过。最终流水线的 Web 类型、认证回归、真实浏览器恢复、生产构建、包体与制品验证通过；检查了桌面登录页、1440px 本地合成控制台及最终 500px 暗色登录页，无横向溢出和错误提示。开发自测与上线回读完成，独立业务验收未执行；真实用户密码登录和实际收款未在本轮执行，支付激活仍按下方阻塞处理。证据入口：开发机 `~/.local/state/lmm-api/native-access-web-20261117/`，服务器访问配置备份 `/root/.local/state/lmm-api/native-access-repair-20261117T1040Z/`。

- **插件模型目录修复（已推送并发布，2026-11-17）**：`packages` 六个提供方插件与安装器已统一采用账户目录发现和 models.dev 自动缓存/模糊元数据匹配，删除静态模型白名单；补修大目录限制、零容量记录、Magpie 原生能力/缓存价格映射与安装说明。实际 Magpie `0.1.2512` 安装最终 OpenCode 原生/默认入口及 Pi 包，均列出完整语料的 **4,002 / 4,002** 个支持路由，0 遗漏/意外新增，417 条明确专用类型不注册为聊天模型。该数字来自完整公共语料与本地 OAuth 夹具，不是用户生产账户数量；付费推理为 0。七个 GitHub Release 已发布，Pi npm `alpha` 已为 `0.1.0-alpha.6` 并逐字节读回；其它 npm/Marketplace 渠道限制、制品摘要和全部指针见[插件发布验收](validation/plugin-catalog-2026-11-17.md)。

- **控制台页面修复（已随本轮 Web 部署）**：[a82094e](https://github.com/TokenNotIncluded/lmm-api-source/commit/a82094e71ba706e47230cd4d4edd2261c4a7e6ea) 的个人资料可选模块局部不可用提示与显式重试、插件可用性和依赖状态、主机与进程指标归属、客户端配置步骤及状态检测采样时间修正，均已包含在顶部记录的固定 Web 版本中；概览用量与紧凑用户列表也随同发布。相关测试类型问题已在后续提交修复，本轮部署的全局 Web 类型检查通过。渠道/管理员模型卡片、目录预览及任务/绘图原生记录仍为开发机未提交候选。历史缓存与延迟字段需要独立第 76 组件和离线白名单补录，当前生产数据尚未完成该项回填；此前隔离 PostgreSQL 捕获与完整浏览器矩阵未形成最终结果的范围继续保留为未完成。上述后端与数据候选未因本轮静态 Web 部署获得上线资格。原开发证据入口：`~/.local/state/lmm-api/native-ui-parity-20261117/`、`native-console-repair-20261117/`、`channel-ui-completion-20261117/`。

- **生产日志、模型定价与原始配置补齐（已应用并完成真实网页验收，2026-11-17）**：生产源按只读一致性快照采集于北京时间 02:03:56；04:01:30 再核对日志数、最大日志 ID/时间及用户数量，均未变化。通过 [c50157a](https://github.com/TokenNotIncluded/lmm-api-source/commit/c50157a6baa52c8472f4eb220f4dd3add95f7348) 修复定价页面数据入口，并在当前用量为空而同条件历史查询有数据时自动选择历史记录，保留用户手动选择。历史迁移与严格历史身份识别见 [4d4948d](https://github.com/TokenNotIncluded/lmm-api-source/commit/4d4948d5383db47538dbefc2b80536e1b4a2f602)。原 197,991 条日志逐行 raw SHA 和完整投影不变，精确新增 1,337 条，最终 199,328 条、2 个 sources、2,502 个完整 scopes。生产另有 4 条零 quota/零 token 的孤儿日志，因其用户在源库中不存在而明确排除，未伪造归属。补入的 2 个身份仅供历史关联，用户/账户均 inactive、登录选项关闭，没有凭据、会话、开账、资金或新请求。实际候选 Core 财务查询排除这 2 个历史身份后，missing/invalid_usage/invalid_wallets 均为 0。
  定价与原始配置实现见 [3c5617e](https://github.com/TokenNotIncluded/lmm-api-source/commit/3c5617ef0d1f17eaefb70514b4a36a600f525e8f)，该批静态 Web 当时由 [43933971760](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43933971760) 发布。使用真实管理员会话完成正式预览与确认发布，pricing/routing 版本现为 2/2，650 条规则与标准化候选逐字段相等；原 99 条精确保留，新增 551 条，20 条含大整数的规则无精度损失。原始价格快照保留 22 项配置（含 ModelRatio 的 3,907 个原始条目）、23 个启用渠道的非秘密元数据，并列出 95 个未转换为通用价格规则的模型/分组及原因；其中 4 个已有既存图像定价覆盖。这 95 项只保留原值供核对，不能据此声称全部已经支持新计费。新价格规则不自动增加可用上游或路由；既有路由内容和上游凭据保持一致。
  已写入并逐字节回读 HeaderNavModules、Notice、console_setting.announcements、legacy_pricing_snapshot.v1 四键。81 张金融表的记录数和完整行摘要均不变；身份会话凭据表的行数与摘要在验收期间有变化，已单独记录，不能把它描述成完全未变；上游凭据表则已证明保持一致。付款、OAuth、原生运行配置和访问边界按各自现行状态保留，本轮不启用实际收款。
  运行 Core 使用固定 1eba304 基座加本轮 3 个 Core 文件，未直接部署包含其他并行候选的整个 main。74 个数据库组件声明与旧制品逐字相同，manifest SHA256 为 `d193076302254b791db8815bc294944b05fa6dd6f05cd0484d66137f4b98cab6`；实际 ELF SHA256 为 `cb3d2a567cc8d724f1368959169eff94a150395ee37f62620ec2ab62eb0c0178`。OpenSSL 4 两库封装在该 Core 的专用目录，仅此服务设置库搜索路径；实际映射、重定位、内置 provider 与 TLS context probe 均通过，系统 OpenSSL 3 保留。04:04:52–04:05:48 暂停 native 共 56 秒，依次提交身份、历史、四键配置后恢复；旧 Go 进程未变。初次 SQL 文件读取权限失败发生在执行前，改为 stdin 传入同 SHA 原 SQL 后全部提交。停机时完整备份的目录及完整解压已验证，尚未执行生产恢复演练。
  最终真实页面：[使用日志](https://native.api.lmm.best/usage-logs/common) 自动切到历史来源，当前日期显示 15 条并包含最新记录；[模型定价](https://native.api.lmm.best/system-settings/billing/model-pricing) 显示 650 条，原始配置面板显示 22/23/95 与采集时间。Core ready=200、check-db 通过；没有发起真实模型或支付请求。历史 planner/PG、财务 PG、四键事务 PG、3 项 Core snapshot 测试和 Web 定向测试/类型/构建均通过。独立只读验收完成于 04:08:11。
  私有证据目录：`~/.local/state/lmm-api/native-data-refresh-20261117/`。历史全量核验 `history-identity-postverify.v3.live.private.result.json` SHA256 `3412364fcf64fe8cca407a233ad2042341566b24f883ef23158f6ec7b5e23d9e`；价格/配置/金融核验 `migration-final-pricing-config-finance-verification.private.json` SHA256 `42fbd033acde2b4831c7dd065fc2da1c72391a5c8419b2f7e9a8291e87aaa614`。原始日志、数据库备份、秘密、会话材料与含个人信息截图未提交仓库。此条更新运行 Core 版本与本轮数据验收状态，下面其他任务的历史收据仍按各自时点理解。

- **钱包与账号密码登录补修（此前部署记录，当前 Web 版本见上）**：钱包修复 [5d3a7b1](https://github.com/TokenNotIncluded/lmm-api-source/commit/5d3a7b13db115d23b56fc2094be987b5b9da49eb) 直接使用原生订阅读取，移除旧订阅和旧 topup-info 探测；订阅、奖励等次要区块按需挂载，已保存操作自动展开恢复。渠道或订阅读取失败留在所属区块，提供显式重试，不冒充空数据。余额突出显示有明确 v2 元数据支持的参考金额，同时保留精确 Credits；付款金额和到账额度继续使用冻结的 Core 合同。手机布局 [30ca41f](https://github.com/TokenNotIncluded/lmm-api-source/commit/30ca41fcf128d709bdc96255056683f7b8eeefa7) 将辅助统计收紧为一行。37 项钱包定向自测、类型检查、生产构建和包体检查通过；本地四组浏览器流程及两组最终手机布局通过，资金场景验证不确定结果后刷新只查询原订单、不重放付款。测试均使用合成账户和订单，不是实际收款验收。
  [部署 43930294571](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43930294571) 将固定 30ca 静态 Web 发布至 arch-shadow；首页、登录、钱包、状态及 7 个资源返回 200，未登录管理员接口 401，旧站 200，公网和回环 index 摘要一致。随后并行定价任务的 [部署 43933971760](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43933971760) 推进到 3c5617e / 0.0.0-preview.9；本轮 23 个钱包相关文件相对 30ca 无变化，当时 index SHA256 为 `02cb521964e8b93179260bbc9785f3a4711938363a8e36c077a6db27690d668c`。已发布 30ca 制品的三组受控浏览器场景通过；手机恢复的网络空闲等待及缺失 HumanGrant fixture 失败保留，制品变更时哈希守卫拒绝继续，不放宽版本绑定。当前固定 3c 制品的手机账号密码登录与原订单恢复回读通过；111 次实际联网均为本站静态 GET，全部账号/支付 API 使用合成响应，刷新未增加报价和确认之外的两次合成写入。最终回执为 `browser-wallet-deployed-3c5617-mobile-resume-run.json`。Core PID、Extensions PID 与支付配置摘要保持原值。证据：父仓库 `.git/lmm-wallet-20261117/`；独立业务验收未执行。

- **账号密码链路（同现网 Core 制品的隔离 HTTP 验证通过）**：Web auth 53 项定向自测通过；现网合成不存在账号的错误密码返回 401、无刷新 Cookie。使用匹配现网 1eba 的 ELF 和该制品初始化的隔离 74 组件数据库，正确密码登录、读取本人、刷新令牌/Cookie、登出、旧令牌及旧 Cookie 拒绝均通过，使用合成账号并已清理自有进程与数据库。这不覆盖真实用户会话、历史 bcrypt 导入升级或线上 HTTPS 全流程。两个当前源码 Rust 登录测试在编译 780 秒后超时，未进入测试，仍为 NOT_RUN。实际运行 ELF 的 schema 为 74；配置目录名 native69 不是当前 schema 证据。回执：开发机 `~/.local/state/lmm-api/password-core1eba-http-20261117/handoff.json`。

- **真实收款阻塞与权限候选（源码已推送，未部署）**：一次只读 provider 查询确认 5001/5003 共用 store，唯一生产 webhook 仍为 `https://api.lmm.best/api/waffo-pancake/webhook/prod`，进入正式旧 Go；SDK checkout 无每单通知地址，不能用浏览器 SuccessURL 替代回调。运行 Core/Extensions 的资金写权限、渠道及回调仍关闭。当前运行 Core 的 native/legacy 证明和入账共用权限，Go 还可恢复持久化旧回调。候选 [b8c5998](https://github.com/TokenNotIncluded/lmm-api-source/commit/b8c5998bed3893fbe1e39e690d45677a7c364f26) 新增默认 false 的 `legacy_proof_ingress` 与 `legacy_credit_evidence`，分离实际 RPC gate 和 capabilities，保留共同只读查询；配置样例和升级说明同步。格式、文档和差异检查通过，300 秒定向编译超时，未执行测试；整库存在并行输入变化，本轮七文件冻结摘要一致。Go 装配不强制 legacy 写能力，无需修改 Go。现网 1eba 不认识新字段，不能仅写配置先行启用；该候选没有编译/运行通过或上线资格。
  SDK CNY 数量报价序列化与原始 webhook 路由两个 Go 定向测试通过，使用合成 transport/Core fixtures，不代替实际账本闭环。已准备 native 精确回调和独立 store/新渠道的审阅候选，未修改 provider 注册或生产资金。仍需商户隔离决策、匹配制品的报价→checkout→签名回调→唯一入账/重启恢复验证，以及资金配置预览确认。证据入口：`payment-candidate/`、`provider-read/result.json`、`permission-split/final-receipt.json`，均位于本轮父仓库私有证据目录。

- **首页与浏览器会话恢复补修（此前部署记录，现行 Web 版本见上）**：固定 [9b34c07](https://github.com/TokenNotIncluded/lmm-api-source/commit/9b34c07bf1e0c45f06be18d9cbf31aedb11e05ca) 将公共页头统一为单一可读字标，移除首页重复 Logo，AI 导航改为与正文对齐的轻量链接，保留键盘、滚轮和触摸入口；手机目录边线错位已修正。恢复提示改为紧凑中性样式；修复崩溃后 pending 状态无法进入可恢复不确定态，以及应用启动清理提前删除会话恢复标记的两处问题，继续要求新文档、只读服务端证明与用户显式重新登录，不重放中断请求。上线回读另发现未迁移的脚本列表返回 503，已将该可选查询的失败与显式重试留在脚本区，停止全局 toast 与自动重试，保留服务端拒绝。67 项定向测试、Chromium 受控 HTTP 的 16 组协议场景、5 组首页/登录布局、2 组完整应用恢复场景及 2 组脚本局部失败/显式重试布局通过，类型、范围 lint、生产构建与包体检查通过；首次失败和修复证据保留。[部署运行 43914606492](https://github.com/TokenNotIncluded/lmm-api-source/actions/runs/43914606492) 仅更新 `arch-shadow` 的静态 Web，实际制品 `0.0.0-preview.6` 与提交相符，公网与回环 index SHA256 均为 `8b50917caef7a3e50939c1bdeedb9b6ce679b6bdab3f0492ab3967f09a9344a7`；首页、登录页、状态接口及 7 个脚本/样式资源均为 200，未登录管理员接口仍为 401，旧站首页为 200，Core PID 与运行配置摘要未变。隔离匿名浏览器在当前 Core 上完成 pending 转 uncertain、新文档只读 reconcile 200、显式重新登录及标记清除，确认恢复前无 refresh 重放；该结果不覆盖已有账号的完整登录与续期组合。同包历史功能前端不据此取得后端部署或完整业务验收资格。开发与上线回读证据在父仓库 `.git/lmm-ui-20261117/`；独立业务验收未执行。

- **Google 登录按用户要求暂停**：用户明确先使用账号密码登录，待切换正式域名后再接入 Google。原私有 provider 候选未激活，OAuth 配置未变更。此前 native 回调的 redirect_uri_mismatch 诊断保留；正式域名确定后再核对最终 callback、client 配对与真实授权。历史证据：开发机 `~/.local/state/lmm-api/native-google-restoration-20261117/`，仓库不记录秘密或会话材料。

- **Native 入口 403 已恢复（2026-11-16，访问配置修复）**：公网诊断请求与 Nginx 的 access forbidden by rule 记录对应，当前操作者出口未被旧 IP 白名单覆盖；Core、前端文件及回环 HTTPS 原本正常。仅在 `/etc/nginx/conf.d/lmm-native-shadow.conf` 的 native 域名下新增一条当前操作者允许规则，保留共享 operator ACL 与 deny-all，QA 配置不变。Nginx 配置检查及热重载均退出 0，Core PID 未变化。开发机经公网复验首页、登录页、状态接口及 7 个脚本/样式资源均为 200；未登录管理员接口仍返回 JSON 401，旧站首页为 200。该入口继续采用操作者白名单策略，不代表公开开放或新版本上线。服务器备份与收据：`/root/.local/state/lmm-api/native-access-repair-20261116T153930Z/`；开发机结果：父仓库 `.git/native-403-20261116-verification.json`。

- **管理员用户详情统计合并（源码已推送，未部署）**：[a19e12fd](https://github.com/TokenNotIncluded/lmm-api-source/commit/a19e12fdbd4b7de96952d5136cc01456b43fbfc5) 保留个人资源用量与资金来源统计，另列调用者（actor）范围的模型活动、邀请与导入原始计数；模型活动包含团队资源调用及可用导入历史，与个人用量可能重叠，分开展示、不作总计。累计推广奖励、当前推广余额与历史原始值也保持各自来源和单位。数据库 SUM 超过 i64，以及用户 ID `9007199254740993` 实际结算后详情拒绝的回归均已由失败转为通过：聚合返回精确十进制字符串，详情专用读取支持合法大 ID；旧数字 API 边界及金额、退款、账本命令和收据守卫不变。隔离 PostgreSQL 定向通过管理员 36 项、community 6 项和原有 usage 集成 22 项，all-targets Clippy `-D warnings`、Rust 格式与差异检查通过；Web 28 个文件、175 项通过，格式、版权、lint、类型、生产构建及包体检查通过，gzip 589276 字节。本地浏览器覆盖 60 个布局状态、普通用户拒绝访问 1 项与展开明细 9 项，共 70 个状态；普通角色 debug URL 入口的初次失败与修复后的 clean-route 403 分别保留。视觉与交互检查使用本地只读 synthetic fixtures，尚未完成管理员浏览器到真实 Core、数据库的整链独立验收。

- **管理员用户详情接入原生用量（源码已推送，未部署）**：[733948f](https://github.com/TokenNotIncluded/lmm-api-source/commit/733948f8f165e5bd5da7bf06da8241fad53f3f9c) 按目标用户及其个人资源归属读取 Core 原生请求，汇总已结算、待结算请求数以及结算额、退款额和净消费，并区分个人钱包、团队代付、订阅与赞助。团队归属 Key 与历史导入记录不计入个人原生用量。详情在同一只读一致快照中读取，复用账本收据校验，累计金额以 u128 计算并返回十进制字符串；无用量与统计不可用保持不同状态。Web 使用摘要加折叠明细，七语言各新增 10 个词条并通过 helper 同步。最终 Core 定向 5 项、Web 定向 35 项、Core all-targets Clippy、相关 Rust 格式、Web 类型、范围 lint 与全量格式检查通过；生产构建及首屏包体检查通过，gzip 587314 字节（上限 786432）。本地 Chromium 的 1440/390/320px、英中俄、明暗主题共 8 组检查通过，形成 16 个摘要/展开截图，未见横向溢出；页面使用合成只读 fixture，数据库验证使用隔离 PostgreSQL 18.6，尚未运行真实管理员会话到数据库的整链路独立验收。首次失败与修复证据保留。

- **Responses 与当前历史迁移仍有未闭合范围**：既有 responses_continuity_independent_store_tristate_and_exact_refusal 在隔离 PostgreSQL 18.6 中精确运行通过；旧 CI 的失败没有完整断言差异，不能认定已修复，本轮未据此修改产品或测试逻辑。当时的社区导入仅接受冻结 Core55/56，Core75 的完整历史导入资格尚未闭合；当前 Core77 的连接历史接入与定向验证见本页最新条目。冻结旧合同继续保留，全部历史数据迁移资格仍需单独完成。原生订阅空态的格式错误已在 [0f72236](https://github.com/TokenNotIncluded/lmm-api-source/commit/0f72236e38c2db17e0adda5bdfb59e7ed886eb36) 修正。

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

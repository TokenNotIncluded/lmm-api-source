# Rust 原生账号与身份

Core 拥有账号、浏览器会话、Key、OAuth 授权和申诉；Go 不得直连身份数据库或取得密码验证器、Session 秘密。当前安装与验收状态统一见[新架构进度](progress.md)，本页维护接口、配置和恢复边界。

`lmm-core init-db` 只接受全新空库，重复安装拒绝；普通启动不执行 DDL。`IdentityStore::init_database()` 仅安装身份叶表，Key 管理、用户投影和模型预留还依赖 `abuse.sql` 等完整 Core 组件；缺表或缺权限必须报错。现有账号、Key、余额与旧 schema 使用另行审核的迁移或升级流程，不能靠改指纹、重跑初始化或 staged delta 升级。版本/指纹检查验证安装合同，不是任意人工改表的完整检测。

当前源码的 identity 契约为 v6；MFA 引入阶段的 v5、OAuth 早期的 13 组件记录均有各自历史范围，不作为当前完整库的组件总数。私密文件、服务账户和备份权限统一遵循[文件系统规则](filesystem-layout.md#权限与服务账户)。

## 账号与权限分开

`accounts.id` 是资源与资金归属标识，`users.id` 是登录人标识，`teams.id` 是公开团队标识。个人用户与团队各自指向独立账号。平台角色 `user`、`admin`、`superadmin` 与账号服务等级 L0–L4 分开；界面的 L5/L6 兼容展示不授予团队消费权或提高个人服务权益。

Key 保存 `owner_account_id`，付款候选的外键和顺序在 `key_funding_rules`，账号内订阅/钱包顺序在 `core_billing.account_policies`。个人 Key 默认仅用个人账号，显式配置后才可加入当前获准消费的团队；团队 Key 仅用所属团队。服务凭证及平台管理角色均不能代替消费授权。

请求接纳时必须固定付款账号，校验当前成员身份与消费许可。一笔费用不拆给多个账号，拒绝、数据库故障或未知结果不触发换人付款。预算包含已提交金额及待结算预留，并绑定稳定账号或团队—用户关系；退出重入、换 Key 或角色变化不得清零。资金操作还需独立授权，身份响应只提供当前权限快照。

## 历史管理员的单独离线激活

A2 身份导入保留历史管理员停用及运行时密码登录关闭状态。导入事务保存来源摘要、迁移记录、物理目标、登录名和密码验证器摘要；来源禁用、删除、未知验证器或禁止密码登录时拒绝激活。不得直接改 `active` 绕过批准，也不得把导入后的关闭状态误读为来源禁止登录。

导入、规划和应用均实际读取 `pg_catalog.pg_control_system()`。集群管理员须在审核后的离线准备中仅向安装 owner 授予其 `EXECUTE`；runtime 不得获得该权限。owner 连接的 `lmm.migration_instance_nonce` 必须匹配目标回执中的 64 字符十六进制 nonce，不能使用请求自报的物理身份。命令不补权限或生成 nonce。

两个命令均由安装 owner 使用独立配置离线执行，从 stdin 读取上限 64 KiB 的 JSON，不启动服务或补装 schema：

| 命令 | 请求与结果 |
| --- | --- |
| `plan-admin-activation` | 读取一个用户的导入事实，返回 candidate 和 `confirmation_sha256`，不应用。请求含 `user_id`、64 字符十六进制 `approval_id`、`approver`、Unix 秒 `expiry_unix`、`policy_revision` 和完整权限政策。 |
| `apply-admin-activation` | 操作者核对来源、目标、权限、有效期后，提交 `{"confirmation_sha256":"…","approval":{…完整 candidate…}}`；仅有用户 ID 不足以批准。未知、重复字段及缺失/未知权限均拒绝。 |

政策固定包含 `ChannelRead`、`ChannelOperate`、`ChannelWrite`、`ChannelSensitiveWrite`、`ChannelSecretView`、`AcquisitionRead`、`AcquisitionWrite`、`AcquisitionDetails`、`AcquisitionExport`，每项显式 `allow` 或 `deny`。`superadmin` 对这些已知动作保留角色优先级，普通 `admin` 的显式 `deny` 优先；团队消费仍需独立授权。

应用事务原子保存政策、激活单个账号、打开允许的密码登录、撤销旧凭据，并将认证版本增加一次；MFA 要求和角色保持来源值。同一已应用批准只读重放。runtime 只能读取必要政策，不能写导入事实、政策或批准回执，也不能通过普通身份表写入创建特权角色或启用未批准管理员。输入及批准材料先设 `umask 077`，不提交 Git。

该入口使用已审核来源事实。导入管理员目录读取检查完整政策；其余细粒度管理接口、真实操作者批准及旧站权限等价仍按各自验收范围推进。

## 邮箱验证码、绑定与密码重置

Core 的加密持久出箱和 SMTP worker 负责邮件，使用正式 identity v6 新库及匹配运行授权。已有 `[identity]` 可配置：

```toml
[identity]
email_config_file = "/etc/lmm.conf.d/email.toml"
email_verification_enabled = true
```

要求注册验证邮箱时才设置 `email_verification_enabled = true`；缺邮件配置会拒绝启动。仅配置邮件也可用于绑定与找回密码。侧文件示例：

```toml
challenge_key_id = "proof-v1"
proof_key_file = "/etc/lmm.conf.d/secrets/email-proof"
rate_key_file = "/etc/lmm.conf.d/secrets/email-rate"
active_key_id = "mail-v1"
reset_origin = "https://app.example.com"
local_http = false

[key_files]
mail-v1 = "/etc/lmm.conf.d/secrets/email-aead"

[smtp]
host = "smtp.example.com"
port = 587
from = "noreply@example.com"
tls = "starttls"
username = "smtp-user"
password_file = "/etc/lmm.conf.d/secrets/smtp-password"
```

proof、限流及各出箱加密密钥必须彼此独立，文件含非零原始 32 字节，权限为 `0400`/`0600`，不可为软链接。SMTP 用户名和密码文件同时提供或省略；TLS 接受 `starttls`/`implicit`。仅显式本地测试可用 `plaintext_loopback_test`，且仅数字回环地址、禁止认证。`local_http = true` 也限本地测试；存在 Web 配置时，其 origin/local_http 必须一致。Legacy 环境模式用 `LMM_CORE_EMAIL_CONFIG_FILE`，不混用 TOML 来源。

Web 接口为 `POST /api/identity/email/verification/request`、`POST /api/identity/email/password-reset/request`、`POST /api/user/register`、`POST /api/oauth/email/bind`、`POST /api/user/reset`；原生接口在 `/core/v1/identity/email/`。邮件适配器不重新开放 Web 安装已关闭的原生注册入口，法律同意、会话、近期认证和 MFA 校验继续生效。

验证码请求的统一接纳响应不泄露账号资格或承诺投递。消费证明、账号修改及必要通知在同一事务提交。重置链接为配置站点的 `/user/reset#email=...&token=...`，保留原证明和有效期；Web 在启动前清除 URL 证明。worker 退出时停止新领取并等待当前发送。SMTP 最终 250 仅记录 `accepted_by_smtp`；丢失回执保持 UNKNOWN，不自动重发为成功。

## 外部登录与配置

### Google 内置登录预设

Google 使用内置 OAuth 预设，不再作为新建自定义提供者模板。已有自定义记录、绑定、Client ID、密钥和用户不自动删除或迁移；内置浏览器入口实际可用时才替换登录按钮，未配置时保留自定义回退。配置来自 Core 只读 TOML 与私有密钥文件，管理页仅展示固定端点、scope 和回调说明，不经 `/api/option` 假装保存成功。

在完整 Core 配置中保留数据库、Web、RPC 等设置，并显式只读挂载密钥：

```toml
[identity.google]
client_id = "registered-client-id.apps.googleusercontent.com"
client_secret_file = "/run/secrets/google-client-secret"
redirect_uri = "https://api.example.test/api/oauth/google/callback"
```

Google 应用中登记的回调必须等于 `[identity.web] origin` 加 `/api/oauth/google/callback`。原生 `/core/v1/auth/oauth/google/callback` 是另一种安装方式，不启用浏览器预设。公开状态仅在浏览器桥接匹配时提供 `google_oauth=true` 和公开 Client ID，不返回密钥。

浏览器经同源 `POST /api/oauth/google` 提交法律协议确认，再使用固定 Google 授权端点与 PKCE。回调验证签名、issuer/audience、nonce、一次性 state 和 HttpOnly 浏览器绑定，签发既有 cookie 家族并固定跳转 `/console`。用户停用、MFA 与注册限制不得绕过；未接通本地第二因素续接的安装必须失败关闭。真实 Google 账号与浏览器组合需各自验收。

### OIDC 设置保存

Web 表单仅在启用 OIDC 且 discovery URL 非空，或修改该 URL 且没有同时停用 OIDC 时，先验证绝对 HTTP(S) URL 并获取 discovery 文档，再保存选项。成功后保存 trim 后的 URL；空 URL 保留手动端点模式。保存其他提供者、未变更表单或停用 OIDC 不获取未变更的 discovery，停用也不受旧无效 URL/供应商不可用阻碍。

discovery 失败会选中 OIDC 页签，在 URL 字段显示既有本地化错误，并停止全部选项提交。后续 option API 仍逐项更新，该行为没有使其他保存失败变成事务。保存与重置按钮在 discovery 和全部更新完成前保持禁用；提供者凭据、Telegram 字段、回调和注册限制沿用原合同。

实现及本地 fixture 测试位于 `apps/web/src/features/system-settings/auth/oidc-discovery.ts` 和 `oidc-discovery.test.ts`，覆盖保存决策、请求时序、输入不变、规范化及失败。提供者可用性和完整登录验收见[进度](progress.md)。

## 客户端 OAuth2 授权

可选 Rust issuer 为 `lmm` CLI 提供只读授权；有限预算 Pi 模型委托另行显式配置。完整协议及来源边界见 [Core OAuth 合同](../services/core/oauth-foundation.md)。

| 客户端 | 必需 scope | 额外条件 |
| --- | --- | --- |
| `lmm` | 恰好 `catalog:read balance:read` | 只读目录与个人余额；不签发 Key 或授予开发者资格。 |
| `lmm-pi` | `catalog:read balance:read usage:read models:invoke` | 已配置有限 `pi_max_credits` 及实际 ModelGateway，用户已有真实 `developer_access`，同意不超过安装上限的整数 credits 预算。管理员身份不能替代资格。 |

真实登录及 MFA 后，用户显式 Continue，核对账号和 group 快照，再 Allow/Deny；签发 scope 另含对应 `group:`。安装的 groups 只是上限，仍检查当前开发者许可和各 group 授权。客户端可申请已知的可选 MCP/remote scope，本 issuer 不发行它们，也不授予团队、管理员、支付、转账或市场权限。

预备 GET 不自动授权。每个不可变阶段使用独立 HttpOnly cookie 和一次性 CSRF，延迟响应仅清自己的 cookie；Core 同时验证持久绑定、原生 Session、当前权限与 MFA。Web refresh cookie 路径保持 `/api/user/auth`。回调只含 code/state/iss 或 error/state/iss，access/refresh 不进入 URL。

### 安装与资源

在已有受限 Core TOML 中增加下列片段，并保留 Web origin/refresh 私钥。issuer 必须与 HTTPS Web origin 完全相同，禁止 HTTP、IP、端口或代理头推断：

```toml
[identity.oauth2]
issuer = "https://api.example.invalid"
groups = ["default"]
access_ttl_seconds = 900
family_ttl_seconds = 2592000
# 同时配置 [model] 后，可显式开放 Pi 有限预算同意。
# pi_max_credits = 1000000
```

access TTL 为 1–86400 秒，family TTL 为 1–31536000 秒，refresh 不延长 family。完整安装包含 `oauth-grants`/`abuse`；由 `deployment/postgres/grant-core-runtime.sql` 的 `enable_oauth2=true` 准备最小 DML，缺少或过宽权限拒绝启动，不用 owner 凭证运行。恢复须保留整个 Core 数据库、消费/撤销 lineage 及 Web/MFA 私钥，不能只恢复余额或删除已用 refresh 记录。

发现端点为 `/.well-known/oauth-authorization-server`、`/.well-known/oauth-protected-resource/api/oauth2`；受保护读接口为 `/api/oauth2/catalog`、`/api/oauth2/balance`，Pi 另有 `/api/oauth2/usage`。目录使用实际网关同一 serving/price 快照，缺失/失效返回 unavailable，合法空目录如实返回。余额保留整数 credits、单位和开账来源，无美元换算证明时不可用。

授权码一次消费并精确绑定 client/resource/loopback redirect/S256 verifier。refresh 只保持或缩小权限，旧 refresh 重放持久撤销整个 family；未知提交结果不允许盲重试。Core 重检当前用户、权限、epoch、consent 和到期，普通浏览器 refresh 独立于 OAuth family。`/api/oauth2/revoke` 撤销当前 client 的整个 family，未知合法 token 幂等成功。

### Pi 模型与财务来源

Pi 使用真实 Bearer access token 进入同一 Chat/Responses JSON/SSE 网关。`X-LMM-Group` 为规范无 padding base64url group 选择器，重复或超出同意范围拒绝。预留、开始及追加预算的同一数据库事务重检 family/access、本人账号、身份版本、开发者资格、group、期限和有限预算。

OAuth 财务记录使用 `key_id=null` 及不可变 OAuth 来源；原生 Key 整数与旧 JSON receipt 保持。不创建假 Key、不信任客户端来源 JSON，不使用他人或团队资金。同一 family 的预算覆盖 refresh 前后全部 access、已消费净额及未结算预留，本人/账号/套餐限制继续生效。

撤销、禁用或标记阻止新消费，合法旧预留仍由原 payer 结算、退款或恢复。开始失败或缺少供应商最终 usage 不猜零消费。usage 只读同 family、本人与个人 payer 的真实来源账单：在一个可重复读快照中按 keyset 批次读取最近 31 天；`items` 累计序列化大小超过 16 MiB，或校验/资源失败时返回 unavailable，不静默截断。Web 日志保留真实 null Key ID。实现见 `services/core/src/usage.rs::read_oauth_usage`。

### 浏览器授权管理

个人资料的 “Authorized clients” 卡片只展示客户端、用途、scope、预算及有效期，不返回 access/refresh、digest 或模型请求原文：

| 方法与路径 | 作用 |
| --- | --- |
| `GET /api/user/auth/oauth2/grants` | 最多 100 个当前账号浏览器授权族的脱敏摘要。 |
| `DELETE /api/user/auth/oauth2/grants/{family_id}` | 撤销本人一项授权。 |
| `POST /api/user/auth/oauth2/grants/revoke-all` | 撤销本人全部浏览器授权。 |

同源请求使用 HttpOnly/SameSite 会话，继承 Origin/Referer 检查。撤销事务重新锁定用户、个人账号和 `auth_version`，family ID 仅作定位，不能跨账号撤销。下一次授权检查拒绝该 family 的 access/refresh；执行中请求仍按账本及排空规则收尾。客户端收到 `401 invalid_token` 后清除本地令牌并要求重新授权，禁止把 bearer 复制到页面、日志、URL 或客服消息。

## 可选 TOTP 二次验证（MFA）

源码提供密码登录后的 TOTP 挑战、一次性恢复码，以及浏览器启用、停用和恢复码重建。MFA 默认不配置；在既有完整配置中增加以下片段，用户仍须完成设置与验证码确认：

```toml
[identity.mfa]
keyring_file = "/etc/lmm.conf.d/secrets/core/mfa-keyring.toml"
```

keyring 是独立 TOML：

```toml
version = 1
active_key_id = "mfa-key-1"

[[keys]]
id = "mfa-key-1"
key_file = "/etc/lmm.conf.d/secrets/core/mfa-key-1.bin"
```

操作者准备专用随机密钥，每个文件恰好 32 个原始字节，不用十六进制/Base64/带换行文本，不复用 Web refresh 或 Key 加密密钥。keyring 和 key 均使用绝对路径的普通文件，路径无 `.`、`..` 或软链接，Unix 权限仅 `0400`/`0600`；容器须显式只读挂载每个引用文件。秘密不进入仓库、包、URL、命令行或日志。

keyring 上限 16 KiB，`version=1`，`keys` 为 1–8 项。ID 唯一，含 1–64 个 ASCII 字母、数字、`_` 或 `-`；`active_key_id` 必须存在。未知字段或格式错误拒绝启动。配置 `version=1` 与 identity schema 版本各自独立。

Core 启动时加载 keyring，新秘密用活动 ID 加密，旧密文按原 ID 解密。更换活动 ID 不重加密历史；备份必须保留旧密钥和映射，当前没有自动轮换/重加密工具。`two_factor_enabled` 只表示进程已加载 keyring；要求 MFA 的账号缺配置或可解密因素时返回 `MFA_UNAVAILABLE`，不降级为密码登录。

Google 第二因素续接、Passkey、管理员重置、旧 TOTP/恢复码迁移及密钥恢复分别验收。已有 `second_factor_required` 不能通过关闭策略、补造会话或替代管理员绕过。

## 账号停用与申诉

停用代际、所有权证明、申诉及审核事务由 Core 持有。停用用户可用既有密码或已绑定浏览器身份验证，取得十分钟 HttpOnly `lmm_account_appeal` Cookie，路径限 `/api/user/account-action-requests`。它只允许申诉及读取回执，不签发 Session、Key、refresh family 或绕过普通接口 MFA；已关闭账号及特权管理员恢复使用独立流程。

| 接口 | 合同 |
| --- | --- |
| `GET /api/user/account-action-requests/appeal` | 返回已验证上下文及已有回执；缺失/过期证明返回 401，不尝试普通凭据 refresh。 |
| `POST /api/user/account-action-requests/appeal` | 使用 Cookie 提交 `{reason}`，或停用密码所有者提交 `{username,password,reason}`；必须为配置 Origin 的 JSON，用户名本身不构成证明。 |
| `GET /api/account-action-requests` | 管理员列表支持 `status`（`pending`、`approved`、`rejected`、`superseded`、`all`）、`page`、`page_size`（最大 100）；原数组 envelope 带含总数的 `pagination`。 |
| `POST /api/account-action-requests/{id}/{approve\|reject}` | 当前管理员 Session，提交 `{note,expected_revision,expected_auth_version}`；版本必须为规范十进制字符串。 |

原因限 5–2000 字符且最多 8192 UTF-8 字节。每个停用代际仅一项 pending；并发/重试返回原始原因，不覆盖证据，同一证明提交不同证据冲突。用户只看到匹配停用记录的显式 `public_reason`；管理员私有 `reason` 和审核 `note` 保密，历史缺公开原因时如实显示未公布。

审核仅作用于普通用户及未变更的停用代际。后续安全变更使 pending 成为 `superseded`；过期或冲突审核返回 409，完全相同审核重放仅返回回执。批准在同一事务启用用户、推进身份代际、保存决定和审计，不恢复被撤销的 Session/refresh/Key；用户须重新登录并满足当前 MFA。拒绝保持停用，两种决定均不修改账本或重开已关闭账号。

### 申诉安装与固定升级合同

`account-appeals` 是第 42 个组件，随完整新装安装。既有精确 41 组件 PostgreSQL 18 前驱使用 `core-account-appeals-v1`；精确 32 组件前驱使用 `core-functional-completion-v1`，两者冻结后继均为 44 组件。该数字描述升级合同，不是当前完整新装总数。过程名称和前驱指纹保持不变，旧 42/43 后继请求不批准 44 后继。

采用匹配 release manifest、实测 catalog target、维护与角色围栏流程，再执行安装者授权脚本和 runtime `check-db`。runtime 对 requests 仅有 SELECT/INSERT 及审核列 UPDATE，flow UPDATE 仅限 `request_id`，不得改所有权、proof digest、代际或原因；升级回执仅 owner 可写。离线升级保留前驱行、序列、owner 与 ACL，不自动扩大 runtime 权限。实现见 `services/core/src/core_schema/upgrade/account_appeals.rs` 和 `completion.rs`。

本地 DB/HTTP 结果、浏览器验收和生产上线分别记录；issue #971 中具体账号的实际停用与案件审核仍由生产操作者处理。

## 创建测试用户并验证 Key

先按 [Docker 指南](../deployment/docker/README.md) 创建隔离开发栈，并在当前 Bash 定义 `dc`。`bootstrap-user` 仅离线创建新用户，不覆盖已有用户，会话只输出一次；不得作为网页注册、登录或 OAuth 接口公开。

```sh
# 当前 shell 已设置 umask 077；拒绝覆盖现有凭证文件。
(set -C; printf '%s\n' '{"user_id":1,"platform_level":1}' |
  dc --profile tools run --rm -T core-init bootstrap-user \
  > deployment/docker/.secrets/identity-session.json)
```

示例把会话及 Key 写入私有文件，不放入命令行、URL 或终端输出：

```sh
python3 - <<'PYKEY'
import json
from pathlib import Path
from urllib.request import Request, urlopen
root = Path('deployment/docker/.secrets')
key_path = root / 'identity-key.json'
if key_path.exists():
    raise SystemExit('Test key file already exists.')
session = json.loads((root / 'identity-session.json').read_text())
request = Request('http://127.0.0.1:8080/core/v1/keys',
    data=json.dumps({'owner': {'kind': 'personal', 'id': 1}, 'ttl_seconds': 3600}).encode(),
    headers={'Authorization': 'Bearer ' + session['secret'], 'Content-Type': 'application/json'})
with urlopen(request, timeout=10) as response:
    issued = json.load(response)
with key_path.open('x') as file:
    file.write(json.dumps(issued))
key_path.chmod(0o600)
request = Request('http://127.0.0.1:8080/core/v1/identity',
    headers={'Authorization': 'Bearer ' + issued['secret']})
with urlopen(request, timeout=10) as response:
    identity = json.load(response)
assert identity['user_id'] == 1
assert identity['funding_account_ids'] == [identity['owner_account_id']]
print('Test key authorized with its personal account.')
PYKEY
```

HTTP `owner.id` 使用带类型的公开标识：个人为用户 ID，团队为团队 ID，不填内部 `owner_account_id`。身份响应另返回内部账号 ID；Protobuf `AccountRef.id` 的原义不变。

## 已接通接口

| 接口 | 规则 |
| --- | --- |
| `GET /core/v1/identity` | 会话或 Key，读取身份及当前付款顺序。 |
| `POST /core/v1/keys` | 会话；`owner`、`ttl_seconds`、可选 `funding_policy`；新凭证只返回一次。 |
| `DELETE /core/v1/credentials/{id}` | 会话，仅撤销本人凭证，可重复。 |
| `POST /core/v1/teams` | 会话，每人最多一个团队。 |
| `GET /core/v1/teams?after=0` | 会话，团队 ID 分页，每页最多 100 项。 |
| `POST /core/v1/teams/{id}/invites` | 会话，接收人 ID、角色、消费许可及有效期。 |
| `POST /core/v1/invites/accept` | 接收人会话、JSON `token`；重检双方权限和版本。 |
| `DELETE /core/v1/teams/{team}/members/{user}?version=1` | 会话及预期成员版本，旧版本拒绝。 |

请求使用单个 `Authorization: Bearer ...`，响应禁止缓存，JSON 上限 16 KiB，错误不暴露数据库详情。会话/邀请最长 7 天，Key 最长 365 天，数据库只存摘要。成员退出保留授权版本，重入不恢复旧 Key/邀请；成员变更与审计同事务，审计失败回滚。

浏览器兼容入口还提供 `GET /api/user/sessions`、`DELETE /api/user/sessions/{sid}`、`POST /api/user/sessions/revoke-others`，分别列出本人原生会话、撤销本人指定会话和撤销其他本人会话；`PUT /api/user/sessions/settings` 仅接受布尔 `session_auto_logout`。这些入口要求当前浏览器凭据、认证证明和当前 MFA，API Key 不能代替，不导入旧会话。

撤销当前会话清除 refresh Cookie；每周退出策略持久保存，关闭策略不复活已过期会话。列表不含凭据/刷新令牌，未采集 IP/User-Agent 返回 `null`。

## 验证和未完成项

Console 公告设置的可写键、权限、原子审计、并发和未知提交语义另见 [Console public content settings](core-console-settings.md)。公开内容保存不授予宿主或资金配置写权限。

测试 `DATABASE_URL` 必须指向独立 PostgreSQL，角色可创建临时库。SQLx 建立隔离测试库后由测试显式安装，缺库失败而不跳过。这些命令供对应完整身份验收使用，局部文档维护不运行：

```sh
(cd services/core && cargo fmt --all --check)
(cd services/core && cargo clippy --locked --all-targets -- -D warnings)
(cd services/core && cargo test --locked --all-targets)
mise exec -- just test-docker \
  "${LMM_DOCKER_TEST_OUTPUT:?Set a new empty private absolute output directory}"
```

既有身份测试范围包含非空库/并发安装、版本不符、账号和付款规则、权限分离、邀请/退出重入、审计回滚、凭据到期/撤销和 DB 故障。已记录的 Pi 编译与独立 PostgreSQL/TCP 验收覆盖来源绑定、跨续期预算、撤销竞争、用量和退款，上游为受控本地服务；各记录仅适用于其固定组合。

真实 HTTPS CLI/Pi SDK、系统凭据库、browser/MFA/多标签、模型协议与正反例、余额并发、安装/独立升级、加密备份/空目标恢复仍须对应验收；scope 不代替实际模型支持。旧候选的目录证据限本机 loopback Chat/Responses、相同公开/上游模型名及受限价格，外部 HTTPS、模型名重映射和图片能力按具体组合验证。市场 OAuth/MCP 与 marked L0 honey-only Pi 需要独立来源和授权，普通模型委托不得绕过标记。

Google 外部凭据、MFA 续接、Passkey及旧身份迁移的当前范围以[进度](progress.md)为准。`/health/ready` 依赖受限 runtime 和启用组件；模型默认关闭，readiness 不代替供应商调用和扣费验收。完整方法见[质量指南](quality.md)。

# 新架构影子站高频更新

在 CI/CD 合同内对 **native 影子站**（operator 自备的非生产入口）做高频、可回滚更新。  
不自动切换正式生产入口；正式切域见 [core-cutover-preflight](core-cutover-preflight.md) 与生产授权。  
域名、主机名与备份路径由部署方配置，本仓库不绑定任何具体站点。

## 合同边界

- 自动允许：路径命中的 microkernel / MK 门禁（push/PR）。
- 影子自动部署：[`deploy.yml`](../.github/workflows/deploy.yml) 接受手动触发及 `v*` / `shadow-v*` tag；服务器、组件、目录和探针由环境变量描述，见[通用部署配置](../deployment/automation/README.md)。普通 main push 不部署，不执行生产切流。
- 必须手动：`ci.yml`、`linux-packages.yml`、`microservice-distribution.yml`、`microservice-images.yml`、`server-release-qualification.yml`（仅 `workflow_dispatch` / `workflow_call`）。
- 禁止：CI 直推生产、工作流硬编码域名、无授权资金动作。
- 拓扑断言：`node --test scripts/workflow-topology.test.mjs`。

## 自动更新

配置 `DEPLOY_TARGETS`、`DEPLOY_ENVIRONMENT`、`DEPLOY_KNOWN_HOSTS` 和环境 Secret
`DEPLOY_SSH_PRIVATE_KEY` 后，手动运行 `deploy.yml`，或推送来源属于 main 的
版本 tag。工作流构建固定源码的所选组件，验证归档，按目标顺序部署并检查实际运行
进程/静态页面。Core 更新前只执行 `check-db`；不自动升级 schema。

运行主机只在事务期间保留当前和候选两个固定槽位；成功立即删除旧槽位和上传包，
失败回滚通过后删除候选。中断后下一次更新清理非活动候选。回滚失败时保留必要的
两份恢复材料并报失败，不继续后续服务器，也不积累按版本命名的历史目录。

## 手工组件更新

1. 本地定向自测 → 提交并推送 `main`。
2. 确认 GitHub Actions 已启用；路径命中的门禁应自动跑。
3. 手动调度打包门：`gh workflow run linux-packages.yml --ref main`（需要时再 `ci.yml` / `microservice-distribution.yml`）。
4. 用固定 revision 构建组件产物，`build-distribution.py verify` 通过后，只更新影子主机上的对应组件。
5. 回读：服务 `active`、二进制 SHA、本机端口探针；正式生产进程与入口状态不变。
6. 回滚材料落本机统一备份目录并校验，不在运行主机堆历史副本。

## 本机组件更新（Extensions 示例）

```sh
SOURCE_SHA="$(git rev-parse HEAD)"
OUT="${LMM_HF_OUT:-$HOME/.local/state/lmm-api/hf-update-$SOURCE_SHA}"
mkdir -p "$OUT/extensions" "$OUT/distributions" "$OUT/evidence"
(cd services/extensions && CGO_ENABLED=0 GOOS=linux GOARCH=amd64 \
  go build -mod=readonly -trimpath -ldflags='-s -w' \
  -o "$OUT/extensions/lmm-extensions" ./cmd/extensions)
python3 scripts/build-distribution.py build \
  --component extensions --version 0.1.20-hf --revision "$SOURCE_SHA" \
  --platform linux-amd64 --input "$OUT/extensions" --output "$OUT/distributions"
python3 scripts/build-distribution.py verify \
  "$OUT/distributions"/lmm-extensions-*-linux-amd64.tar.gz --revision "$SOURCE_SHA"
```

影子安装（仅 Extensions，不重启 Core/DB，不碰正式生产进程）：

1. 备份当前 `lmm-extensions` 二进制到本机备份目录。
2. `scp` 新 ELF → 影子主机临时路径，再 `install` 到其服务单元声明的路径。
3. 重启对应 Extensions 服务单元；确认 `active` 与 SHA。
4. 探针用本机回环上的 Extensions/Core 监听端口（以影子 TOML/`ss` 为准）；公网影子入口仍受 operator ACL。

Core / Web 按 [release-architecture](release-architecture.md) 各自构建与安装；Core 更新不得顺带重灌库。

## 切域门槛（未齐则停）

- [ ] `main` 与父 gitlink revision 与运行产物一致
- [ ] 固定包 verify 通过；影子全路径业务回读通过
- [ ] `core-cutover-preflight` READY 且 `safe_to_cutover=true`（只读门，不等于批准）
- [ ] 增量同步与停写演练完成；回滚材料在本机
- [ ] 明确授权后，才切换正式生产入口

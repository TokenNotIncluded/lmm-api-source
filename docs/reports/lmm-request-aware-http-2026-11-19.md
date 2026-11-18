# LMM API 请求能力路由：开发机 HTTP 实测

日期：2026-11-19（Asia/Taipei）

## 结果

通过 ssmcp 连接开发机，执行真实 TCP/HTTP 集成验证：**3 项通过，0 项失败，退出码 0**。验证调用生产 `ModelGateway::router()` 的真实监听端口，并由真实 PostgreSQL 处理身份、资金预留与结算。供应商端使用本地测试 HTTP 服务。

测试源码已提交为 [234643a1](https://github.com/TokenNotIncluded/lmm-api-source/commit/234643a101dcd0bc400b01a0c95cb0ad7ed852bd)，验证基线为 `4c2b2a3203d6803312cf24d3a193da783dc8547f`。本轮只新增定向测试，业务路由源码与上一轮 [1831c35b](https://github.com/TokenNotIncluded/lmm-api-source/commit/1831c35bf7c047c6af59491ce6a4ce1f06a81c08) 的修复摘要一致。

## 实际请求与数据库结果

每个场景使用独立测试数据库，请求均为 `POST /v1/chat/completions`，`max_tokens=129`。

| 场景 | HTTP 结果 | 实际发送记录 | 资金结果 |
| --- | --- | --- | --- |
| A 上限 128，B/C 可处理 4096，次数上限 1 | 200 | A 无发送记录；B 命中 1 次；C 命中 0 次；发送编号为 1 | 1 笔预留、1 笔结算；最终扣 10 测试 credits |
| 次数上限 2，B 的 TCP 端口拒绝连接 | 200 | B 发送尝试编号 1，记录 `not_sent`；C 命中 1 次，编号 2 | 仍只有 1 笔预留、1 笔结算；最终扣 10 测试 credits |
| A/B/C 上限均为 128 | 503，`route_unavailable` | 0 条发送记录 | 0 条扣费请求、0 笔资金预留、0 条计费账本记录 |
| 第一场景重复提交相同幂等键 | 409 | B 仍只命中 1 次，C 仍为 0 | 没有第二笔预留或扣费 |

成功场景中，数据库实际记录的预留金额为 **1,048,576 测试 credits**，结算金额为 **10**，预留最终关闭，剩余冻结金额为 **0**。每个场景的初始余额为 2,000,000，成功后可用余额为 1,999,990；拒绝场景的余额保持 2,000,000。复式账本的不平衡分录组数均为 0。

这些金额来自本地测试费率：每个输入 token 收取 1 测试 credit，输出不收费。测试上游报告输入 10、输出 20，因此实际结算 10；预留使用配置的输入上限 1,048,576。它们不代表线上费用或真实供应商报价。

## 测试方法

客户端使用 reqwest 通过 TCP 发送请求；Core 使用实际路由、鉴权、重试、计费和账本实现。B 的失败由专用临时端口关闭后的真实连接拒绝触发，没有直接调用内部 `failed()` 注入结果。

数据库由驱动新建，绑定私有回环端口并使用随机 SCRAM 凭据。安装和合成资金初始化使用独立 owner 连接；HTTP 网关使用 `NOSUPERUSER`、`NOCREATEDB`、`NOCREATEROLE`、`NOBYPASSRLS` 的受限运行角色。测试结束后 PostgreSQL 停止成功，临时目录已清理。

A 保留 HTTPS 外部模型合同的 128-token 输出上限。B/C 的 numeric-loopback 上游使用既有本地策略下的全局 4096 上限，确保测试确实覆盖请求能力筛选。生产 TOML 不允许混合这两种端点模式；本测试通过公开网关构造器设置夹具，没有修改生产配置校验、TLS 或 DNS 策略。

## 命令与证据

```sh
cargo test --locked --offline --jobs 1 \
  --manifest-path services/core/Cargo.toml \
  --test request_aware_http -- \
  --test-threads=1 --nocapture
```

运行环境为开发机 Linux x86_64、`rustc 1.100.0 (0ef1bc386 2026-11-09)`，复用 Cargo 缓存，开发与测试 debug 信息关闭，增量编译关闭。驱动清除继承的数据库环境变量后注入专用连接。

执行时间为 **02:39:39–02:40:12（Asia/Taipei）**。Cargo 构建用时 25.80 秒，3 项测试用时 4.81 秒，驱动记录 31.982 秒。首次执行即通过；构建期间曾等待共享 package cache 文件锁，没有测试失败。

验证前后，Core、数据库授权脚本及测试文件共 **1,091 项输入**的摘要保持一致。实际执行的测试程序为 `request_aware_http-f6029becc5881df6`，SHA-256 为：

```text
82080944e499b72e8331948939f8449522c9612c9b5ede150cea71ae59fb940d
```

[结构化证据](lmm-request-aware-http-2026-11-19.evidence.json)保留各场景状态码、发送记录、请求标识、上游命中、账本数值、命令、起止时间和关键输入摘要。[测试源码](../../services/core/tests/request_aware_http.rs)可复用同一套断言。完整本地运行日志和输入清单位于本工作区 `scratch/request-aware-tcp-20261119/`。

## 验证范围

本轮通过的是开发机上的 Core 网关 HTTP 集成实测，覆盖本次路由修复及其资金行为。生产 `lmm-core` 主程序的 TOML 启动、真实付费供应商兼容性、线上部署与独立业务验收未执行。当前项目状态仍统一维护于 [docs/progress.md](../progress.md)。

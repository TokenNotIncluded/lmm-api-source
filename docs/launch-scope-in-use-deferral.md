# Launch-scope-v2：8 个在用模型书面延期

首发以 `launch-scope-v2` 为准：只发布合同齐全能力。下列 8 个模型在 30 天消费窗口内仍有调用，但计划层缺合同/上下文，**不纳入本次首发路由发布**；材料保留在源归档与延期清单，不静默丢弃。

| 模型 | 30 天调用（约） | 主要缺口 | 首发处理 |
| --- | ---: | --- | --- |
| `gpt-image-2` | 35（近 7 天 32） | 缺可安装图片能力进 launch 路由（`image-2` 组未进 launch-scope）；provider/price 上下文未绑定 | 延期；优先下一轮。演练已证明 piqode ch8 图片路径可完成结算，但不能替代正式合同发布 |
| `gpt-image-2.5-flare` | 7 | 同上；Source 图片合同包状态为 `SOURCE_IMAGE_CONTRACTS_REBOUND_NOT_INSTALLABLE` | 延期；演练冒烟曾经语义别名跑通 flare，正式语义名仍缺 |
| `codex-auto-review` | 36 | runtime evidence / provider context / Responses WS 映射缺失 | 延期 |
| `chengzhi-c-no-flash` | 3 | runtime evidence / provider context / price / legacy wire 合同缺失 | 延期 |
| `chatgpt-image-latest` | 1 | 图片合同要求独立 expression/budget；provider/price 未齐 | 延期 |
| `Bouquet` | 1 | runtime evidence / provider context / price / legacy wire 合同缺失 | 延期 |
| `amazon/nova-2-lite-v1` | 1 | runtime evidence / price 合同缺失 | 延期 |
| `xingchen-agi/xing4.0-29b-a4b:free` | 1 | runtime evidence / provider context 缺失 | 延期 |

证据索引（本机私有，不入库）：

- `routing412-preparation-tmernihc/launch-scope-deferred-v2.private.json`
- `routing412-preparation-tmernihc/model-gaps-by-reason.private.json`
- `routing412-preparation-tmernihc/used79-progress-v2.private.json`
- `routing412-preparation-tmernihc/image-capability-contracts412.private.json`
- `routing412-preparation-tmernihc/recent-model-group-channel-30d.private.json`

已覆盖首发冒烟的在用能力：`gpt-6.1-sol`（Responses）、`google/gemini-3.1-flash-image`（目录语义；演练图片上游经 piqode）。OpenRouter 对 gemini/gpt-5 图片上游仍可能 402，不作为取消延期的理由。

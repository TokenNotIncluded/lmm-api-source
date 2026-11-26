# 工具市场 MCP 客户端迁移

把 MCP 连接地址设为 `https://<市场域名>/mcp/market`，在客户端选择 OAuth 登录。登录后刷新工具列表，选择唯一的 `metamcp`，将调用内容放入字符串参数 `code`。以下示例中的域名、工具 ID、版本 ID 和请求 ID 均需替换成该实例的实际值。

## 连接与授权

1. 移除这个连接中手动设置的 API key、个人市场令牌、Cookie 和身份请求头。使用支持 Streamable HTTP 和 OAuth 的 MCP 客户端重新连接。首次请求收到 HTTP 401 时，从 `WWW-Authenticate` 的 `resource_metadata` 读取受保护资源元数据。
2. 客户端按元数据中的 `authorization_servers` 查找授权服务器。实现自有客户端时，从授权服务器元数据读取注册、授权和令牌端点。按需要注册公共客户端，使用授权码、PKCE S256、注册的回调 URI 和随机 `state`。在授权和换取令牌的请求中都传入元数据给出的完整 `resource`。旧连接中的 `?mode=full` 不属于资源标识。
3. 用户在浏览器中确认客户端、范围和授权预算。客户端把收到的访问令牌放入 `Authorization: Bearer <access_token>`，按 MCP 协议初始化连接，然后调用 `tools/list`。令牌过期时带上原客户端 ID、规范资源 URI 和刷新令牌续期，并保存服务端返回的新令牌。需要重新授权时，让用户重新登录。

| 客户端操作 | OAuth 范围 |
| --- | --- |
| 查看目录、工具详情、本人调用状态 | `market:discover` |
| 调用已有授权的工具 | `market:discover market:invoke` |
| 为当前客户端加载或卸载工具 | `market:discover market:manage` |
| 建立工具授权、设置工具或客户端预算 | `market:discover market:invoke market:manage` |

HTTP 403 且挑战包含 `insufficient_scope` 时，按挑战列出的范围重新取得用户同意。调用工具还需该用户和客户端对目标版本的有效授权。通过 OAuth 登录本身不会购买工具或确认扣款。

## 替换调用格式

原来发送 `{"action":"search","query":"drawing"}` 的客户端，现在发送下面的 JSON-RPC 请求。搜索只读。响应中的 `result.structuredContent.items` 可以为空。

```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"metamcp","arguments":{"code":"search({query:'drawing',limit:1})"}}}
```

从搜索结果的 `items[].id` 取服务 ID，通过 `details({service_id:'服务 ID'})` 查看具体工具、输入结构、版本和报价。调用时使用所选工具的 `tool_id` 和 `version_id`，按它的输入结构填写 `arguments`。下面的模板对应一个业务参数为空的工具。

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"metamcp","arguments":{"code":"invoke({tool_id:'TOOL_ID',version_id:'VERSION_ID',request_id:'UNIQUE_REQUEST_ID',arguments:{}})"}}}
```

`code` 支持一个动作，或用 `let` 将只读动作的结果传给后续动作。每个动作接收一个对象，一段程序最多八个动作，修改状态或实际调用工具只能放在最后。客户程序从 `tools/list` 返回的 `_meta["lmm/actions"]` 读取各动作的字段定义。为动态内容生成表达式时，应先用 JSON 序列化函数编码字符串和对象。

需要确认的调用应保存原始 `request_id`、参数和服务端返回的确认状态。按客户端提示取得用户确认后，把 `requestState` 和 `inputResponses` 放回 `tools/call.params`，与 `arguments` 并列。收到调用 ID 后，用 `call_status({call_id:'CALL_ID'})` 查询原调用。遇到超时或状态未知，保留原请求 ID 和参数；生成新请求 ID 会表示另一笔操作。

## 旧客户端看到的响应

| 请求 | HTTP / MCP 响应 | 客户端处理 |
| --- | --- | --- |
| 个人市场令牌作为 Bearer | HTTP 401，`WWW-Authenticate` 带 `resource_metadata`、`market:discover` 和 `invalid_token` | 删除旧令牌，启动 OAuth 登录 |
| 仅发送 `X-Api-Key`，或没有凭据 | HTTP 401，挑战给出资源元数据和发现范围 | 改用元数据所列的授权服务器 |
| 同时发送有效 OAuth Bearer 和 `X-Api-Key` | HTTP 400 | 移除额外的身份头和 Cookie |
| 使用有效 OAuth，向 `?mode=full` 或 `?mode=compact` 发 `tools/list` | HTTP 200，仅列出 `metamcp` | 可以将地址简化为 `/mcp/market`；刷新缓存的旧工具名 |
| `metamcp` 的参数仍是 `{"action":"search"}` | HTTP 200，`result.isError=true`，文本说明需要 `code` 并给出搜索示例 | 替换成上面的 `arguments.code` |
| 直接调用当前目录没有的工具名，例如 `lmm_market_search` | HTTP 200，JSON-RPC `error.code=-32602`，消息指出未知工具名 | 先枚举工具，再用 `metamcp` 查询或调用目标 |

客户端应分别检查 HTTP 状态、JSON-RPC 的 `error` 和 MCP 工具结果的 `isError`。随后按原调用回执区分待确认、处理中、完成和失败。

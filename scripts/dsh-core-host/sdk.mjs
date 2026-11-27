// CI-only Cordis assembly. The stock DSH plugin fixes its issuer to api.lmm.best.
// Use its real credential bridge, provider profile and browser auth implementation
// with the vendored provider's explicit issuer constructor and the stock DSH SDK.
import assert from "node:assert/strict";
import { join } from "node:path";
import { createModels } from "@earendil-works/pi-ai";
import { credentialKey } from "@deepseek-ai/dsh-credentials";
import { PiAiAdapter } from "@deepseek-ai/dsh-llm-pi-ai";
import {
  CatalogCoordinator,
  credentialStoreFrom,
  DSH_CLIENT_ID,
  lmmProfile,
  resolveDshCapabilities,
} from "pinned-dsh-plugin";
import { LmmIntegration } from "pinned-lmm-integration";
import { mountBrowserAuth } from "pinned-dsh-web-auth";

export const name = "lmm-dsh-sdk-core-fixture";
export const inject = ["llm", "credentials", "authorization", "connection"];

export function apply(ctx) {
  const issuer = process.env.LMM_DSH_SDK_ISSUER;
  assert.equal(issuer, "https://pi-issuer.example.test");
  const integration = new LmmIntegration({
    issuer,
    clientId: DSH_CLIENT_ID,
    hostName: "DSH",
    capabilities: resolveDshCapabilities,
    refreshJournalDirectory: join(process.env.DSH_HOME, "sdk-refresh-journal"),
  });
  const key = credentialKey(name, "lmm");
  const credentials = credentialStoreFrom(ctx, key);
  const authContext = {
    env: async () => undefined,
    fileExists: async () => false,
  };
  const models = createModels({ credentials, authContext });
  models.setProvider(integration.provider);
  const catalog = new CatalogCoordinator(models);
  const profiles = new Map([["lmm", lmmProfile(integration)]]);
  const adapter = new PiAiAdapter({
    profiles: () => profiles,
    resolveApiKey: async () => undefined,
    auth: { credentials, authContext },
    resolveAttachments: () => ctx.get("attachments"),
  });
  ctx.llm.registerAdapter(["lmm"], adapter);
  ctx.authorization.registerFlow({
    key,
    label: "LMM",
    methods: [{ id: "oauth", label: "Sign in with LMM" }],
    async run(session) {
      await models.login("lmm", "oauth", {
        signal: session.signal,
        notify(event) {
          if (event.type === "auth_url")
            session.notify({
              message: event.instructions || "LMM OAuth",
              url: event.url,
            });
        },
        prompt() {
          throw new Error("unexpected SDK prompt");
        },
      });
    },
  });
  mountBrowserAuth(ctx, key);
  ctx.effect(() =>
    ctx.connection.fetch.register({
      path: "/api/lmm-sdk-core",
      methods: ["POST"],
      requestBody: "buffered",
      async fetch() {
        await catalog.refresh(undefined, true);
        const list = await ctx.llm.listModels("lmm");
        assert.equal(list.length, 1);
        const model = list[0].id;
        const chunks = [];
        for await (const part of ctx.llm.stream({
          provider: "lmm",
          model,
          sessionId: "dsh-sdk-wallet-proof",
          system: "Reply briefly.",
          messages: [
            {
              role: "user",
              content: [{ type: "text", text: "Reply briefly." }],
            },
          ],
          maxTokens: 20,
        }))
          chunks.push(part);
        const terminal = chunks.filter((c) => c.type === "finish");
        assert.equal(terminal.length, 1);
        assert.equal(terminal[0].reason.kind, "stop");
        assert.equal(
          chunks
            .filter((c) => c.type === "text-delta")
            .map((c) => c.text)
            .join(""),
          "local stream answer",
        );
        return Response.json({ model, terminal: true });
      },
    }),
  );
  ctx.effect(() => () => integration.dispose());
}

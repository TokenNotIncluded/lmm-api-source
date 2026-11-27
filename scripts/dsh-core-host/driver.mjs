import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as pause } from "node:timers/promises";
import {
  approveBrowser,
  boundedBody,
  openCoreTls,
} from "../ci/managed-host-fixture.mjs";

const config = JSON.parse(await new Response(process.stdin).text());
assert.equal(config.issuer, "https://pi-issuer.example.test");
const home = await mkdtemp(join(tmpdir(), "dsh-core-sdk-"));
let child, tunnel, base, headers;
let stage = "prepare DSH SDK fixture";
async function stop() {
  if (!child || child.exitCode !== null || child.signalCode !== null) return;
  const ended = once(child, "exit");
  child.kill("SIGTERM");
  await Promise.race([ended, pause(3000)]);
  if (child.exitCode === null && child.signalCode === null)
    child.kill("SIGKILL");
  await ended;
}
async function launch() {
  const cli = join(
    process.env.DSH_SDK_PACKAGE,
    "node_modules/@deepseek-ai/dsh/lib/bin.js",
  );
  child = spawn(process.execPath, [cli, "web", "--no-open", "--port", "0"], {
    cwd: home,
    env: {
      PATH: process.env.PATH,
      HOME: home,
      DSH_HOME: home,
      LMM_DSH_SDK_ISSUER: config.issuer,
      LMM_MODELS_DEV_CACHE: join(home, "models-dev.json"),
      NODE_EXTRA_CA_CERTS: process.env.FIXTURE_TLS_CERT,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let tail = "";
  let url;
  function read(chunk) {
    tail = (tail + chunk).slice(-8192);
    const match = tail.match(/http:\/\/127\.0\.0\.1:\d+\/\?token=[\w-]+/);
    if (match) url = match[0];
  }
  child.stdout.on("data", read);
  child.stderr.on("data", read);
  for (let i = 0; i < 300 && !url && child.exitCode === null; i++)
    await pause(100);
  assert.ok(url, "stock DSH did not expose its authenticated Web endpoint");
  const entry = await fetch(url, {
    redirect: "manual",
    signal: AbortSignal.timeout(10000),
  });
  base = new URL(url).origin;
  const cookie = entry.headers
    .getSetCookie()
    .map((c) => c.split(";")[0])
    .join("; ");
  assert.ok(cookie);
  headers = { cookie, origin: base, "content-type": "application/json" };
}
async function rpc(action, payload = {}) {
  const res = await fetch(base + "/api/lmm-auth/" + action, {
    method: "POST",
    headers,
    signal: AbortSignal.timeout(30000),
    body: JSON.stringify({
      type: "client-request",
      rpcId: "sdk-proof",
      method: "lmm-auth/" + action,
      payload,
    }),
  });
  assert.equal(res.status, 200);
  const { result } = JSON.parse(await boundedBody(res));
  assert.equal(result.ok, true);
  return result.value;
}
try {
  const version = JSON.parse(
    await readFile(
      join(
        process.env.DSH_SDK_PACKAGE,
        "node_modules/@deepseek-ai/dsh/package.json",
      ),
      "utf8",
    ),
  ).version;
  assert.equal(version, "0.2.2-rc.1");
  const profile = join(home, "profiles", "web");
  await mkdir(profile, { recursive: true });
  await writeFile(
    join(profile, "cordis.patch.yml"),
    "- insert:\n    - id: lmm-dsh-sdk-core-fixture\n      name: " +
      JSON.stringify(process.env.DSH_SDK_ASSEMBLY) +
      "\n",
  );
  tunnel = await openCoreTls({
    ...config,
    keyFile: process.env.FIXTURE_TLS_KEY,
    certFile: process.env.FIXTURE_TLS_CERT,
  });
  stage = "official DSH Web startup";
  await launch();
  assert.equal((await rpc("status")).signedIn, false);
  stage = "DSH OAuth interaction";
  const start = await rpc("begin");
  let url;
  for (let i = 0; i < 140 && !url; i++) {
    const view = await rpc("poll", { attempt: start.attempt });
    url = view.notices?.find((n) => n.url)?.url;
    if (!url) {
      assert.equal(view.state, "pending");
      await pause(100);
    }
  }
  assert.ok(url);
  assert.equal(new URL(url).searchParams.get("client_id"), "lmm-dsh");
  assert.equal(new URL(url).searchParams.get("code_challenge_method"), "S256");
  stage = "Core browser consent and PKCE";
  await approveBrowser(url, {
    issuer: config.issuer,
    refresh: config.refresh,
    cap: config.cap,
  });
  let signedIn = false;
  for (let i = 0; i < 140 && !signedIn; i++) {
    const { state } = await rpc("poll", { attempt: start.attempt });
    if (state === "authorized") signedIn = true;
    else {
      assert.equal(state, "pending");
      await pause(100);
    }
  }
  assert.ok(signedIn && (await rpc("status")).signedIn);
  stage = "native credential persistence";
  await stop();
  await launch();
  assert.equal((await rpc("status")).signedIn, true);
  stage = "DSH SDK stream and terminal";
  const response = await fetch(base + "/api/lmm-sdk-core", {
    method: "POST",
    headers,
    signal: AbortSignal.timeout(60000),
  });
  assert.equal(response.status, 200);
  const result = JSON.parse(await boundedBody(response));
  assert.equal(result.terminal, true);
  assert.equal(tunnel.calls.length, 1);
  const [call] = tunnel.calls;
  assert.equal(call.method, "POST");
  assert.equal(call.status, 200);
  assert.equal(call.path, "/v1/chat/completions");
  assert.ok(call.body.includes("[DONE]"));
  assert.ok(
    typeof call.requestId === "string" &&
      call.requestId.length > 0 &&
      call.requestId.length <= 256,
  );
  stage = "match the selected DSH model against the probed Core catalog";
  const access = tunnel.getAccess();
  assert.ok(access);
  const catalog = await fetch(config.issuer + "/api/oauth2/catalog", {
    headers: { authorization: "Bearer " + access },
    signal: AbortSignal.timeout(10000),
  });
  assert.equal(catalog.status, 200);
  const { models } = JSON.parse(await boundedBody(catalog));
  assert.equal(models.length, 1);
  const [row] = models;
  assert.equal(result.model, row.group + " / " + row.upstream_model);
  assert.equal(call.group, row.group_id);
  await rpc("logout");
  assert.equal((await rpc("status")).signedIn, false);
  process.stdout.write(
    JSON.stringify({
      version,
      requestId: call.requestId,
      modelId: row.id,
      group: call.group,
      terminal: true,
    }),
  );
} catch {
  const requests = (tunnel?.calls ?? []).map(({ status, path }) => ({
    status,
    path: path === "/v1/chat/completions" ? path : "other",
  }));
  console.error(
    "DSH SDK acceptance failed during " +
      stage +
      " " +
      JSON.stringify(requests),
  );
  process.exitCode = 1;
} finally {
  await stop();
  await tunnel?.close();
  await rm(home, { recursive: true, force: true });
}

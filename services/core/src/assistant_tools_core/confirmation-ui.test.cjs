// UI unit boundary only: real Session/provider/PG/browser acceptance is separate.
"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const code = fs.readFileSync(__dirname + "/confirmation.js", "utf8");
const id = "a".repeat(64), nonce = "b".repeat(64);
const path = "/api/user/auth/assistant/confirmations/" + id;
const evil = "</pre><img src=x onerror=alert(1)><script>alert(2)</script>\u2028'\"&";
function confirmation(state = "pending") {
  return { tool_name: evil, tool_version: "main-3948a0a1", preview: { title: evil, body: "exact\nmessage" }, expires_at_unix: 2000000000, state, executed: false };
}
async function flush() { for (let i = 0; i < 4; i++) await new Promise(resolve => setImmediate(resolve)); }
function ui(replies, location = path) {
  const nodes = {};
  for (const name of ["status", "details", "preview", "approve", "reject", "two-factor-field", "two-factor-code","key-card","key-status","key-secret","key-reveal","key-refresh","market-limits","market-price","market-price-limit","market-total-limit","market-call-limit","market-lifetime"]) {
    nodes[name] = { disabled: true, hidden: true, value: "", textContent: "", listeners: {}, addEventListener(name, callback) { this.listeners[name] = callback; } };
    Object.defineProperty(nodes[name], "innerHTML", { set() { throw new Error("unsafe HTML sink"); } });
  }
  const requests = [];
  const windowListeners = {};
  const context = {
    document: { getElementById(name) { assert.ok(nodes[name]); return nodes[name]; } },
    window: { location: { pathname: location }, addEventListener(name,callback) { windowListeners[name]=callback; } },
    async fetch(url, options) {
      assert.ok(url === path || url === path + "/key");
      assert.equal(options.mode, "same-origin");
      assert.equal(options.credentials, "same-origin");
      assert.equal(options.redirect, "error");
      assert.equal(options.cache, "no-store");
      requests.push({ url, ...options });
      const reply = replies.shift();
      if (reply instanceof Error) throw reply;
      assert.ok(reply, "unexpected extra request");
      return { ok: reply.status !== false, async json() { return reply.body; } };
    },
  };
  vm.runInNewContext(code, context);
  return { nodes, requests, pagehide() { windowListeners.pagehide(); }, click(name) { nodes[name].listeners.click(); } };
}
test("malicious preview remains exact inert text; load never approves; double click submits once", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, { body: { ok: true, receipt: confirmation("approved"), executed: false } }]);
  await flush();
  assert.equal(view.requests.length, 1);
  assert.equal(view.requests[0].method, "GET");
  assert.equal(view.nodes.preview.textContent, JSON.stringify(confirmation().preview, null, 2));
  assert.ok(view.nodes.details.textContent.includes(evil));
  assert.equal(view.nodes.approve.disabled, false);
  view.click("approve"); view.click("approve"); await flush();
  assert.equal(view.requests.length, 2);
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "approve" });
  assert.equal(view.requests[1].headers["x-lmm-assistant-csrf"], nonce);
  assert.ok(view.nodes.status.textContent.includes("尚不代表业务已执行"));
  assert.equal(view.nodes.approve.disabled, true);
});
test("reject uses the same real browser nonce and closed decision", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, { body: { ok: true, receipt: confirmation("rejected") } }]);
  await flush(); view.click("reject"); await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "reject" });
  assert.equal(view.nodes.status.textContent, "已拒绝。");
});
test("missing nonce, unauthenticated response and invalid path never enable approval", async () => {
  for (const replies of [[{ body: { ok: true, confirmation: confirmation() } }], [{ status: false }], [{ body: { ok: false } }]]) {
    const view = ui(replies); await flush(); view.click("approve"); await flush();
    assert.equal(view.requests.length, 1);
    assert.equal(view.nodes.approve.disabled, true);
  }
  const view = ui([], "/api/user/auth/assistant/confirmations/../../other"); await flush();
  assert.equal(view.requests.length, 0);
  assert.equal(view.nodes.approve.disabled, true);
});
test("an uncertain POST is not retried or reported executed", async () => {
  const view = ui([{ body: { ok: true, confirmation: confirmation(), csrf: nonce } }, new Error("lost ACK")]);
  await flush(); view.click("approve"); await flush(); view.click("approve"); await flush();
  assert.equal(view.requests.length, 2);
  assert.ok(view.nodes.status.textContent.includes("查询原操作状态"));
  assert.equal(view.nodes.approve.disabled, true);
});

test("only a bound execution receipt is reported complete and rendered as inert text", async () => {
  const done = { ...confirmation("approved"), challenge_id: id, operation_id: "c".repeat(64), executed: true };
  done.execution_receipt = { version: 1, executed: true, audit_id: 7, confirmation_id: id, operation_id: done.operation_id, after: { display_name: evil } };
  const view = ui([{ body: { ok: true, confirmation: done } }]);
  await flush();
  assert.ok(view.nodes.status.textContent.includes("操作已完成"));
  assert.equal(JSON.parse(view.nodes.preview.textContent).execution_receipt.after.display_name, evil);
  assert.equal(view.nodes.approve.disabled, true);
  const broken = ui([{ body: { ok: true, confirmation: { ...done, execution_receipt: null } } }]);
  await flush();
  assert.ok(broken.nodes.status.textContent.includes("没有收到成功回执"));
});
test("a factor stays browser-only, failed code can be corrected without automatic retry", async () => {
  const pending = { ...confirmation(), requires_two_factor: true };
  const view = ui([
    { body: { ok: true, confirmation: pending, csrf: nonce } },
    { status: false, body: { ok: false, error: "two_factor_code_invalid", executed: false } },
    { body: { ok: true, receipt: confirmation("rejected") } },
  ]);
  await flush();
  assert.equal(view.nodes["two-factor-field"].hidden, false);
  view.nodes["two-factor-code"].value = "123456";
  view.click("approve"); await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "approve", two_factor_code: "123456" });
  assert.equal(view.nodes["two-factor-code"].value, "");
  assert.equal(view.requests.length, 2);
  assert.equal(view.nodes.approve.disabled, false);
  view.nodes["two-factor-code"].value = "654321";
  view.click("reject"); await flush();
  assert.deepEqual(JSON.parse(view.requests[2].body), { decision: "reject" });
  assert.equal(view.nodes["two-factor-code"].value, "");
});

function createdKeyConfirmation() {
  const done = { ...confirmation("approved"), tool_name: "request_create_key",
    challenge_id: id, operation_id: "c".repeat(64), executed: true };
  done.execution_receipt = { version: 1, executed: true, audit_id: 7,
    confirmation_id: id, operation_id: done.operation_id, action: "create", resource_id: 19,
    after: { id: 19, card_id: id } };
  return done;
}
const cardReady = { state: "ready", credential_id: 19, expires_at_unix: 2000000000, one_time: true };
test("created Key is never fetched automatically; explicit private reveal is once and never in preview", async () => {
  const secret = "lmmk_" + "S".repeat(43);
  const view = ui([
    { body: { ok: true, confirmation: createdKeyConfirmation() } },
    { body: { ok: true, key_card: cardReady, csrf: nonce } },
    { body: { ok: true, key_card: { ...cardReady, state: "consumed" }, api_key: secret } },
  ]);
  await flush();
  assert.equal(view.requests.length, 2);
  assert.equal(view.requests[1].url, path + "/key");
  assert.equal(view.requests[1].method, "GET");
  assert.equal(view.nodes["key-secret"].textContent, "");
  assert.equal(view.nodes["key-reveal"].disabled, false);
  view.click("key-reveal"); view.click("key-reveal"); await flush();
  assert.equal(view.requests.length, 3);
  assert.deepEqual(JSON.parse(view.requests[2].body), { reveal: true });
  assert.equal(view.nodes["key-secret"].textContent, secret);
  assert.equal(view.nodes.preview.textContent.includes(secret), false);
  assert.equal(view.nodes.details.textContent.includes(secret), false);
  assert.equal(view.nodes["key-reveal"].disabled, true);
  view.pagehide();
  assert.equal(view.nodes["key-secret"].textContent, "");
});
test("lost private display ACK only permits a status read, never an automatic second reveal or issue", async () => {
  const view = ui([
    { body: { ok: true, confirmation: createdKeyConfirmation() } },
    { body: { ok: true, key_card: cardReady, csrf: nonce } },
    new Error("lost display ACK"),
    { body: { ok: true, key_card: { ...cardReady, state: "consumed" } } },
  ]);
  await flush(); view.click("key-reveal"); await flush();
  assert.equal(view.requests.length, 3);
  assert.equal(view.nodes["key-secret"].textContent, "");
  view.click("key-reveal"); await flush();
  assert.equal(view.requests.length, 3);
  view.click("key-refresh"); await flush();
  assert.equal(view.requests.length, 4);
  assert.equal(view.requests[3].method, "GET");
  assert.ok(view.nodes["key-status"].textContent.includes("一次显示已消费"));
  assert.equal(view.nodes["key-reveal"].disabled, true);
});
test("private metadata cannot smuggle a secret or another credential into the page", async () => {
  for (const body of [
    { ok: true, key_card: cardReady, csrf: nonce, api_key: "lmmk_" + "S".repeat(43) },
    { ok: true, key_card: { ...cardReady, credential_id: 20 }, csrf: nonce },
  ]) {
    const view = ui([
      { body: { ok: true, confirmation: createdKeyConfirmation() } }, { body },
    ]);
    await flush();
    assert.equal(view.nodes["key-secret"].textContent, "");
    assert.equal(view.nodes["key-reveal"].disabled, true);
    assert.equal(view.requests.length, 2);
  }
});

function marketConfirmation() {
  return { ...confirmation(), tool_name: "connect_market_tool",
    preview: { kind: "market_tool", market: { preview: { request: { max_price_quota: 7 } } } } };
}
test("Market limits require explicit browser fields, never defaults from the model preview", async () => {
  const view = ui([
    { body: { ok: true, confirmation: marketConfirmation(), csrf: nonce } },
    { body: { ok: true, receipt: { ...marketConfirmation(), state:"approved",
      challenge_id:id, operation_id:"c".repeat(64), executed:true,
      execution_receipt:{version:1,executed:true,audit_id:8,confirmation_id:id,
        operation_id:"c".repeat(64),action:"connect",resource_kind:"market_consent",
        resource_id:"d".repeat(64),after:{consent:{id:"d".repeat(64)}}} } } },
  ]);
  await flush();
  assert.equal(view.nodes["market-limits"].hidden, false);
  assert.equal(view.nodes.approve.disabled, true);
  assert.equal(view.nodes.reject.disabled, false);
  view.click("approve"); await flush();
  assert.equal(view.requests.length, 1);
  const values = { "market-price-limit": "7", "market-total-limit": "21",
    "market-call-limit": "3", "market-lifetime": "300" };
  for (const [name,value] of Object.entries(values)) {
    view.nodes[name].value=value; view.nodes[name].listeners.input();
  }
  assert.equal(view.nodes.approve.disabled, false);
  view.click("approve"); await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision:"approve",
    market_limits:{max_price_quota:7,max_total_quota:21,max_calls:3,lifetime_seconds:300} });
});
test("Market invalid limits cannot POST, and rejection never submits financial limits", async () => {
  const view = ui([
    { body: { ok:true,confirmation:marketConfirmation(),csrf:nonce } },
    { body: { ok:true,receipt:confirmation("rejected") } },
  ]);
  await flush();
  for (const values of [["6","21","3","300"],["7","6","3","300"],["7","21","0","300"],
    ["7","21","3","59"],["7.5","21","3","300"],["7","9007199254740992","3","300"]]) {
    ["market-price-limit","market-total-limit","market-call-limit","market-lifetime"].forEach((name,i)=>{
      view.nodes[name].value=values[i];view.nodes[name].listeners.input();
    });
    assert.equal(view.nodes.approve.disabled,true);
    view.click("approve");await flush();assert.equal(view.requests.length,1);
  }
  view.click("reject");await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body),{decision:"reject"});
});

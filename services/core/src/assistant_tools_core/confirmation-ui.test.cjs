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
function ui(replies, location = path, storage = new Map()) {
  const nodes = {};
  for (const name of ["heading","instructions","invitation-preview","invitation-recipient","invitation-code","invitation-expiry","invitation-fallback","confirmation-refresh","status", "details", "preview", "approve", "reject", "two-factor-field", "two-factor-code","key-card","key-status","key-secret","key-reveal","key-refresh","market-limits","market-price","market-price-limit","market-total-limit","market-call-limit","market-lifetime"]) {
    nodes[name] = { disabled: true, hidden: true, value: "", textContent: "", listeners: {}, addEventListener(name, callback) { this.listeners[name] = callback; } };
    Object.defineProperty(nodes[name], "innerHTML", { set() { throw new Error("unsafe HTML sink"); } });
  }
  const requests = [];
  const windowListeners = {};
  const context = {
    document: { body: { classList: { toggle() {} } }, getElementById(name) { assert.ok(nodes[name]); return nodes[name]; } },
    window: { location: { pathname: location }, addEventListener(name,callback) { windowListeners[name]=callback; },
      sessionStorage: { getItem(key) { return storage.get(key) ?? null; }, setItem(key,value) { storage.set(key,value); }, removeItem(key) { storage.delete(key); } } },
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
  return { nodes, requests, storage, pagehide() { windowListeners.pagehide(); }, click(name) { nodes[name].listeners.click(); } };
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

function invitation(state = "pending", preview) {
  return { tool_name: "send_invitation", tool_version: "main-3948a0a1",
    challenge_id: id, operation_id: "invitation-operation",
    preview: preview ?? { kind: "invitation_email_v1", email: "recipient@example.invalid", invitation_code: "Ab12" },
    expires_at_unix: 2000000000, state, executed: false };
}
test("invitation shows all actual fields and local expiry without implementation metadata", async () => {
  const value = invitation();
  const view = ui([{ body: { ok: true, confirmation: value, csrf: nonce } },
    { body: { ok: true, receipt: invitation("approved") } }]);
  await flush();
  assert.equal(view.nodes.heading.textContent, "发送邀请");
  assert.equal(view.nodes["invitation-recipient"].textContent, value.preview.email);
  assert.equal(view.nodes["invitation-code"].textContent, value.preview.invitation_code);
  assert.equal(view.nodes["invitation-preview"].hidden, false);
  assert.equal(view.nodes.details.hidden, true);
  assert.equal(view.nodes.preview.hidden, true);
  const local = new Intl.DateTimeFormat(undefined, {
    year:"numeric",month:"short",day:"numeric",hour:"numeric",minute:"2-digit",second:"2-digit",timeZoneName:"short",
  }).format(new Date(value.expires_at_unix * 1000));
  assert.equal(view.nodes["invitation-expiry"].textContent, local);
  assert.equal(view.nodes["invitation-expiry"].dateTime, new Date(value.expires_at_unix * 1000).toISOString());
  assert.equal(view.nodes.approve.textContent, "同意发送");
  assert.equal(view.nodes.reject.textContent, "取消邀请");
  assert.equal(view.requests.length, 1);
  view.click("approve"); view.click("approve"); await flush();
  assert.equal(view.requests.length, 2);
  assert.deepEqual(JSON.parse(view.requests[1].body), { decision: "approve" });
  assert.equal(view.requests[1].headers["x-lmm-assistant-csrf"], nonce);
  assert.ok(view.nodes.status.textContent.includes("查看发送结果"));
  assert.equal(view.nodes.status.textContent.includes("邮件已发送"), false);
  assert.equal(view.nodes.approve.hidden, true);
  assert.equal(view.storage.size, 0);
});
test("an added invitation field or new kind falls back to the entire inert preview", async () => {
  for (const draft of [
    { ...invitation().preview, message: evil },
    { kind:"invitation_email_v2", email:"recipient@example.invalid", invitation_code:"Ab12", extra:evil },
  ]) {
    const view = ui([{ body:{ok:true,confirmation:invitation("pending",draft),csrf:nonce} }]);
    await flush();
    assert.equal(view.nodes["invitation-preview"].hidden,true);
    assert.equal(view.nodes["invitation-fallback"].hidden,false);
    assert.equal(view.nodes.preview.hidden,false);
    assert.deepEqual(JSON.parse(view.nodes.preview.textContent),draft);
    assert.equal(view.nodes.approve.disabled,false);
    assert.equal(view.requests.length,1);
  }
});
test("bad invitation fields, a changed operation and an impossible execution receipt cannot approve", async () => {
  for (const value of [
    invitation("pending", {kind:"invitation_email_v1",email:4,invitation_code:"Ab12"}),
    invitation("pending", {kind:"invitation_email_v1",email:"recipient@example.invalid",invitation_code:""}),
    {...invitation(),challenge_id:"d".repeat(64)},
    {...invitation(),executed:true},
    {...invitation(),expires_at_unix:"2000000000"},
  ]) {
    const view=ui([{body:{ok:true,confirmation:value,csrf:nonce}}]);
    await flush();view.click("approve");await flush();
    assert.equal(view.nodes.approve.disabled,true);
    assert.equal(view.requests.length,1);
  }
  for (const changed of [
    {...invitation("approved"),operation_id:"another"},
    {...invitation("approved"),preview:{...invitation().preview,email:"another@example.invalid"}},
    {...invitation("approved"),expires_at_unix:2000000001},
    {...invitation("approved"),tool_name:"request_create_key"},
  ]) {
    const view=ui([{body:{ok:true,confirmation:invitation(),csrf:nonce}},new Error("lost ACK"),
      {body:{ok:true,confirmation:changed,csrf:nonce}}]);
    await flush();view.click("approve");await flush();view.click("confirmation-refresh");await flush();
    assert.equal(view.nodes.approve.disabled,true);
    assert.equal(view.requests.filter(x=>x.method==="POST").length,1);
    assert.equal(view.storage.size,1);
  }
});
test("unknown invitation first queries the original operation, then a new explicit decision uses the fresh nonce", async () => {
  const fresh = "c".repeat(64);
  const view=ui([{body:{ok:true,confirmation:invitation(),csrf:nonce}},new Error("lost ACK"),
    {body:{ok:true,confirmation:invitation(),csrf:fresh}},
    {body:{ok:true,receipt:invitation("approved")}}]);
  await flush();view.click("approve");await flush();
  view.click("approve");view.click("reject");await flush();
  assert.equal(view.requests.length,2);
  assert.equal(view.nodes["confirmation-refresh"].hidden,false);
  const [key, raw] = [...view.storage.entries()][0];
  assert.equal(key, "lmm.invitation.uncertain:" + path);
  assert.deepEqual(JSON.parse(raw), {version:1,expires_at_unix:invitation().expires_at_unix});
  assert.equal(raw.includes(nonce),false);
  assert.equal(raw.includes(invitation().preview.email),false);
  view.click("confirmation-refresh");await flush();
  assert.equal(view.requests.length,3);
  assert.equal(view.requests[2].method,"GET");
  assert.equal(view.storage.size,0);
  assert.equal(view.nodes.approve.disabled,false);
  assert.ok(view.nodes.status.textContent.includes("仍待你决定"));
  assert.equal(view.requests.filter(x=>x.method==="POST").length,1);
  view.click("approve");await flush();
  assert.equal(view.requests.length,4);
  assert.equal(view.requests[3].headers["x-lmm-assistant-csrf"],fresh);
  assert.deepEqual(JSON.parse(view.requests[3].body),{decision:"approve"});
  assert.equal(view.requests.every(x=>x.url===path),true);
  assert.ok(view.nodes.status.textContent.includes("查看发送结果"));
  assert.equal(view.storage.size,0);
});
test("a remounted unknown invitation stays read-only until original Core read succeeds", async () => {
  const marker = new Map([["lmm.invitation.uncertain:" + path, JSON.stringify({
    version:1,expires_at_unix:invitation().expires_at_unix,
  })]]);
  const view=ui([new Error("read unavailable"),new Error("still unavailable"),
    {body:{ok:true,confirmation:invitation("approved")}}],path,marker);
  await flush();view.click("approve");view.click("reject");await flush();
  assert.equal(view.requests.length,1);
  assert.equal(view.nodes.approve.disabled,true);
  assert.equal(view.nodes["confirmation-refresh"].hidden,false);
  view.click("confirmation-refresh");await flush();
  assert.equal(view.requests.length,2);
  assert.equal(view.storage.size,1);
  assert.equal(view.nodes.approve.disabled,true);
  view.click("confirmation-refresh");await flush();
  assert.equal(view.requests.length,3);
  assert.deepEqual(view.requests.map(x=>x.method),["GET","GET","GET"]);
  assert.equal(view.requests.every(x=>x.url===path),true);
  assert.equal(view.storage.size,0);
  assert.ok(view.nodes.status.textContent.includes("查看发送结果"));
});
test("expired, malformed or mismatched local markers never act as approval or permanently lock a fresh nonce", async () => {
  for (const raw of [
    "not json", "1", JSON.stringify({version:1,expires_at_unix:1}),
    JSON.stringify({version:1,expires_at_unix:2000000001}),
    JSON.stringify({version:1,expires_at_unix:2000000000,approved:true}),
    JSON.stringify({version:2,expires_at_unix:2000000000}),
  ]) {
    const marker=new Map([["lmm.invitation.uncertain:" + path,raw]]);
    const view=ui([{body:{ok:true,confirmation:invitation(),csrf:nonce}}],path,marker);
    await flush();
    assert.equal(view.requests.length,1);
    assert.equal(view.requests[0].method,"GET");
    assert.equal(view.storage.size,0);
    assert.equal(view.nodes.approve.disabled,false);
    assert.equal(view.nodes.status.textContent.includes("已同意发送"),false);
  }
});
test("cancel invitation uses the original nonce and never includes mutable recipient fields", async () => {
  const view=ui([{body:{ok:true,confirmation:invitation(),csrf:nonce}},
    {body:{ok:true,receipt:invitation("rejected")}}]);
  await flush();view.click("reject");await flush();
  assert.deepEqual(JSON.parse(view.requests[1].body),{decision:"reject"});
  assert.equal(view.requests[1].headers["x-lmm-assistant-csrf"],nonce);
  assert.equal(view.nodes.status.textContent,"已取消这项邀请。");
  assert.equal(view.storage.size,0);
});
test("expired invitation remains readable but cannot submit a new decision", async () => {
  const value={...invitation(),expires_at_unix:1};
  const view=ui([{body:{ok:true,confirmation:value,csrf:nonce}}]);
  await flush();view.click("approve");view.click("reject");await flush();
  assert.equal(view.nodes["invitation-recipient"].textContent,value.preview.email);
  assert.equal(view.nodes.approve.disabled,true);
  assert.equal(view.nodes.reject.disabled,true);
  assert.ok(view.nodes.status.textContent.includes("已过期"));
  assert.equal(view.requests.length,1);
});

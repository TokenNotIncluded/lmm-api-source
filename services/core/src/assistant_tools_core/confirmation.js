"use strict";
(() => {
  const status = document.getElementById("status");
  const details = document.getElementById("details");
  const preview = document.getElementById("preview");
  const approve = document.getElementById("approve");
  const reject = document.getElementById("reject");
  const heading = document.getElementById("heading");
  const instructions = document.getElementById("instructions");
  const invitationPreview = document.getElementById("invitation-preview");
  const invitationRecipient = document.getElementById("invitation-recipient");
  const invitationCode = document.getElementById("invitation-code");
  const invitationExpiry = document.getElementById("invitation-expiry");
  const invitationFallback = document.getElementById("invitation-fallback");
  const confirmationRefresh = document.getElementById("confirmation-refresh");
  const factorField = document.getElementById("two-factor-field");
  const factorCode = document.getElementById("two-factor-code");
  const marketField = document.getElementById("market-limits");
  const marketPrice = document.getElementById("market-price");
  const marketInputs = ["market-price-limit", "market-total-limit", "market-call-limit", "market-lifetime"]
    .map(name => document.getElementById(name));
  let marketRequired = false, marketCeiling = 0, pending = false;
  function marketLimits() {
    const values = marketInputs.map(node => {
      if (!/^(0|[1-9][0-9]*)$/.test(node.value)) return NaN;
      const value = Number(node.value);
      return Number.isSafeInteger(value) ? value : NaN;
    });
    const [price, total, calls, lifetime] = values;
    if (values.some(Number.isNaN) || price < marketCeiling || total < price ||
        calls < 1 || calls > 100 || lifetime < 60 || lifetime > 86400) return null;
    return { max_price_quota: price, max_total_quota: total, max_calls: calls, lifetime_seconds: lifetime };
  }
  function enableDecision() {
    if (!pending || busy || typeof csrf !== "string") return;
    if (invitationMode && (invitationUnknown || Date.now() >= invitationExpires)) {
      disable();
      if (!invitationUnknown) status.textContent = "这项邀请确认已过期。请返回原会话查看状态。";
      return;
    }
    approve.disabled = marketRequired && marketLimits() === null;
    reject.disabled = false;
  }
  for (const node of marketInputs) node.addEventListener("input", enableDecision);
  const path = window.location.pathname;
  let csrf = null;
  let busy = false;
  let needsFactor = false;
  let invitationMode = false, invitationUnknown = false, invitationExpires = 0;
  let invitationOperation = null, invitationFrozen = null;
  const invitationMarker = "lmm.invitation.uncertain:" + path;
  // This marker only prevents another browser submit. It grants no authority:
  // Core still owns the original draft, Session, nonce, expiry and decision.
  function rememberedInvitation() {
    try {
      const raw = window.sessionStorage.getItem(invitationMarker);
      if (raw === null) return false;
      const saved = JSON.parse(raw);
      if (!saved || typeof saved !== "object" || Array.isArray(saved) ||
          Object.keys(saved).length !== 2 || saved.version !== 1 ||
          !Number.isSafeInteger(saved.expires_at_unix) ||
          saved.expires_at_unix * 1000 <= Date.now() ||
          (invitationExpires > 0 && saved.expires_at_unix * 1000 !== invitationExpires)) {
        window.sessionStorage.removeItem(invitationMarker);
        return false;
      }
      return true;
    } catch (_) {
      try { window.sessionStorage.removeItem(invitationMarker); } catch (_) {}
      return false;
    }
  }
  function markInvitationUnknown() {
    invitationUnknown = true;
    try {
      window.sessionStorage.setItem(invitationMarker, JSON.stringify({
        version: 1, expires_at_unix: invitationExpires / 1000,
      }));
    } catch (_) {}
    approve.hidden = true; reject.hidden = true;
    confirmationRefresh.hidden = false;
    confirmationRefresh.disabled = true;
  }
  function clearInvitationUnknown() {
    invitationUnknown = false;
    try { window.sessionStorage.removeItem(invitationMarker); } catch (_) {}
    confirmationRefresh.hidden = true;
  }
  function renderInvitation(confirmation) {
    if (invitationFrozen !== null && confirmation.tool_name !== "send_invitation") throw new Error("changed invitation tool");
    invitationMode = confirmation.tool_name === "send_invitation";
    document.body.classList.toggle("invitation-mode", invitationMode);
    invitationPreview.hidden = true; invitationFallback.hidden = true;
    invitationRecipient.textContent = ""; invitationCode.textContent = "";
    invitationExpiry.textContent = ""; invitationExpiry.dateTime = "";
    preview.hidden = false; details.hidden = false;
    approve.hidden = false; reject.hidden = false; confirmationRefresh.hidden = true;
    if (!invitationMode) return;
    if (confirmation.challenge_id !== path.slice(path.lastIndexOf("/") + 1) ||
        typeof confirmation.operation_id !== "string" || !confirmation.operation_id ||
        (invitationOperation !== null && invitationOperation !== confirmation.operation_id) ||
        confirmation.executed !== false || !Number.isSafeInteger(confirmation.expires_at_unix)) {
      throw new Error("invalid invitation binding");
    }
    const frozen = JSON.stringify({ operation_id: confirmation.operation_id, expires_at_unix: confirmation.expires_at_unix, preview: confirmation.preview });
    if (invitationFrozen !== null && invitationFrozen !== frozen) throw new Error("changed invitation draft");
    invitationOperation = confirmation.operation_id; invitationFrozen = frozen;
    invitationExpires = confirmation.expires_at_unix * 1000;
    const date = new Date(invitationExpires);
    if (!Number.isFinite(date.getTime())) throw new Error("invalid invitation expiry");
    const localTime = new Intl.DateTimeFormat(undefined, {
      year: "numeric", month: "short", day: "numeric", hour: "numeric",
      minute: "2-digit", second: "2-digit", timeZoneName: "short",
    }).format(date);
    heading.textContent = "发送邀请";
    instructions.textContent = "此页只确认收件人和邀请码。邮件需在原会话中单独发送。";
    approve.textContent = "同意发送";
    reject.textContent = "取消邀请";
    details.textContent = "有效期至：" + localTime + "（你的本地时间）";
    const draft = confirmation.preview;
    const keys = Object.keys(draft);
    if (draft.kind === "invitation_email_v1") {
      if (typeof draft.email !== "string" || !draft.email ||
          typeof draft.invitation_code !== "string" || !draft.invitation_code) {
        throw new Error("invalid invitation fields");
      }
      if (keys.length === 3 && keys.every(key => ["kind", "email", "invitation_code"].includes(key))) {
        invitationRecipient.textContent = draft.email;
        invitationCode.textContent = draft.invitation_code;
        invitationExpiry.textContent = localTime;
        invitationExpiry.dateTime = date.toISOString();
        invitationPreview.hidden = false;
        preview.hidden = true; details.hidden = true;
      } else { invitationFallback.hidden = false; }
    } else { invitationFallback.hidden = false; }
    if (confirmation.state === "approved" || confirmation.state === "rejected") {
      clearInvitationUnknown();
      approve.hidden = true; reject.hidden = true;
    } else {
      invitationUnknown = invitationUnknown || rememberedInvitation();
      if (invitationUnknown) {
        approve.hidden = true; reject.hidden = true;
        confirmationRefresh.hidden = false; confirmationRefresh.disabled = false;
      }
    }
  }
  const keySection = document.getElementById("key-card");
  const keyStatus = document.getElementById("key-status");
  const keySecret = document.getElementById("key-secret");
  const keyReveal = document.getElementById("key-reveal");
  const keyRefresh = document.getElementById("key-refresh");
  let keyCsrf = null, keyCredential = null, keyBusy = false;
  function keyUncertain() {
    keyReveal.disabled = true;
    keyCsrf = null;
    keySecret.textContent = "";
    keyStatus.textContent = "尚未收到密钥显示结果。请先查询显示状态；本页不会自动重试或创建另一把 Key。";
    keyRefresh.hidden = false;
    keyRefresh.disabled = false;
  }
  async function cardRequest(method) {
    const headers = { Accept: "application/json" };
    if (method === "POST") {
      headers["Content-Type"] = "application/json";
      headers["x-lmm-assistant-csrf"] = keyCsrf;
    }
    const response = await fetch(path + "/key", { method, headers, mode: "same-origin",
      credentials: "same-origin", cache: "no-store", redirect: "error",
      ...(method === "POST" ? { body: JSON.stringify({ reveal: true }) } : {}) });
    const data = await response.json();
    if (!response.ok || data.ok !== true || !data.key_card || data.key_card.one_time !== true ||
        data.key_card.credential_id !== keyCredential) throw new Error("invalid private key card");
    return data;
  }
  function cardState(card) {
    keyReveal.disabled = true;
    switch (card.state) {
      case "ready": keyStatus.textContent = "Key 已创建。只有本浏览器可以一次性显示，密钥不会进入助手对话。"; break;
      case "consumed": keyStatus.textContent = "这把 Key 的一次显示已消费，服务器不会再次返回密钥。若未收到或未保存，请在凭证管理中停用它并重新创建。"; break;
      case "expired": keyStatus.textContent = "私密卡片已过期。Key 仍可能有效；请在凭证管理中检查并按需停用。"; break;
      case "unavailable": keyStatus.textContent = "Key 当前不可显示。请在凭证管理中检查状态。"; break;
      default: throw new Error("invalid private card state");
    }
  }
  async function loadKeyCard() {
    if (keyBusy) return;
    keyBusy = true;
    keySection.hidden = false;
    keyReveal.disabled = true;
    keyRefresh.hidden = true;
    try {
      const data = await cardRequest("GET");
      if (Object.hasOwn(data, "api_key")) throw new Error("unexpected secret on read");
      cardState(data.key_card);
      if (data.key_card.state === "ready") {
        if (typeof data.csrf !== "string" || !/^[a-f0-9]{64}$/.test(data.csrf)) throw new Error("invalid private nonce");
        keyCsrf = data.csrf;
        keyReveal.disabled = false;
      }
    } catch (_) { keyUncertain(); }
    finally { keyBusy = false; }
  }
  async function revealKey() {
    if (keyBusy || typeof keyCsrf !== "string" || keyReveal.disabled) return;
    keyBusy = true;
    keyReveal.disabled = true;
    keyRefresh.hidden = true;
    try {
      const data = await cardRequest("POST");
      keyCsrf = null;
      cardState(data.key_card);
      if (Object.hasOwn(data, "api_key")) {
        if (data.key_card.state !== "consumed" || typeof data.api_key !== "string" ||
            !/^lmmk_[A-Za-z0-9_-]{20,128}$/.test(data.api_key)) throw new Error("invalid private key");
        keySecret.textContent = data.api_key;
        data.api_key = "";
        keyStatus.textContent = "请现在复制并保存。这是本次唯一的密钥显示；离开页面后不能再次查看。";
      }
    } catch (_) { keyUncertain(); }
    finally { keyBusy = false; }
  }
  keyReveal.addEventListener("click", () => { void revealKey(); });
  keyRefresh.addEventListener("click", () => { void loadKeyCard(); });
  window.addEventListener("pagehide", () => { keySecret.textContent = ""; keyCsrf = null; });

  function disable() { approve.disabled = true; reject.disabled = true; }
  function failure() {
    disable();
    csrf = null;
    factorCode.value = "";
    status.textContent = "无法读取或提交这项操作。会话、权限或预览可能已失效，请返回原会话重新发起。没有收到成功回执时，请先查询原操作状态。";
    if (invitationMode || rememberedInvitation()) {
      confirmationRefresh.hidden = false; confirmationRefresh.disabled = false;
      status.textContent = "尚未查明这项邀请的确认结果。请查询原操作状态；本页不会重复提交或重新发送邮件。";
    }
  }
  function show(confirmation) {
    if (!confirmation || typeof confirmation !== "object" || !confirmation.preview ||
        typeof confirmation.preview !== "object" || Array.isArray(confirmation.preview)) throw new Error("invalid preview");
    details.textContent = "操作：" + confirmation.tool_name + " · 版本：" + confirmation.tool_version +
      " · 有效期至：" + new Date(confirmation.expires_at_unix * 1000).toISOString();
    preview.textContent = JSON.stringify(confirmation.preview, null, 2);
    renderInvitation(confirmation);
    pending = confirmation.state === "pending";
    marketRequired = confirmation.tool_name === "connect_market_tool";
    marketField.hidden = !marketRequired || !pending;
    if (marketRequired && pending) {
      marketCeiling = confirmation.preview?.market?.preview?.request?.max_price_quota;
      if (!Number.isSafeInteger(marketCeiling) || marketCeiling < 0) throw new Error("invalid market quote");
      marketPrice.textContent = "当前每次报价上限：" + marketCeiling + " 积分。确认时会重新核对报价和权限。";
    }
    needsFactor = confirmation.requires_two_factor === true;
    factorField.hidden = !needsFactor || confirmation.state !== "pending";
    factorCode.value = "";
    disable();
    switch (confirmation.state) {
      case "pending":
        status.textContent = invitationMode
          ? (invitationUnknown ? "尚未查明这项邀请的确认结果。请查询原操作状态；本页不会重复提交或重新发送邮件。" : "核对收件人和邀请码后，选择同意发送或取消邀请。")
          : "请核对预览后选择批准或拒绝。";
        break;
      case "approved":
        if (invitationMode) {
          instructions.textContent = "你的同意已记录。邮件发送和发送结果仍由原会话处理。";
          status.textContent = "已同意发送。请回到原会话继续并查看发送结果。";
          break;
        }
        if (confirmation.executed === true) {
          const receipt = confirmation.execution_receipt;
          if (!receipt || receipt.version !== 1 || receipt.executed !== true ||
              typeof receipt.audit_id !== "number" || receipt.confirmation_id !== confirmation.challenge_id ||
              receipt.operation_id !== confirmation.operation_id) throw new Error("invalid execution receipt");
          status.textContent = "操作已完成。下面显示本次执行的原始回执；重复提交不会再次执行。";
          preview.textContent = JSON.stringify({ preview: confirmation.preview, execution_receipt: receipt }, null, 2);
          if (confirmation.tool_name === "connect_market_tool") {
            if (receipt.action !== "connect" || receipt.resource_kind !== "market_consent" ||
                typeof receipt.resource_id !== "string" || !/^[a-f0-9]{64}$/.test(receipt.resource_id) ||
                receipt.after?.consent?.id !== receipt.resource_id) throw new Error("invalid market consent");
            status.textContent = "工具授权已创建，尚未执行工具调用。下面是本次授权的原始回执。";
          }
          if (confirmation.tool_name === "request_create_key") {
            if (receipt.action !== "create" || !Number.isSafeInteger(receipt.resource_id) ||
                receipt.resource_id <= 0) throw new Error("invalid key creation receipt");
            keyCredential = receipt.resource_id;
            void loadKeyCard();
          }
        } else {
          status.textContent = "已批准。这里只记录同意，尚不代表业务已执行；请回到原会话查看处理结果。";
        }
        break;
      case "rejected":
        status.textContent = invitationMode ? "已取消这项邀请。" : "已拒绝。";
        break;
      default: throw new Error("invalid confirmation state");
    }
  }
  async function request(method, decision) {
    const headers = { Accept: "application/json" };
    const payload = decision ? { decision } : null;
    if (method === "POST") {
      headers["Content-Type"] = "application/json";
      headers["x-lmm-assistant-csrf"] = csrf;
      if (decision === "approve" && needsFactor) payload.two_factor_code = factorCode.value;
      if (decision === "approve" && marketRequired) {
        payload.market_limits = marketLimits();
        if (payload.market_limits === null) throw new Error("explicit market limits required");
      }
    }
    const response = await fetch(path, { method, headers, mode: "same-origin", credentials: "same-origin",
      cache: "no-store", redirect: "error", ...(payload ? { body: JSON.stringify(payload) } : {}) });
    factorCode.value = "";
    const data = await response.json();
    if (data.ok === false && (data.error === "two_factor_code_invalid" || data.error === "two_factor_locked")) {
      const error = new Error(data.error);
      error.factor = data.error;
      throw error;
    }
    if (!response.ok || data.ok !== true) throw new Error("request rejected");
    return data;
  }
  async function decide(decision) {
    if (busy || typeof csrf !== "string" || !/^[a-f0-9]{64}$/.test(csrf)) return;
    if (invitationMode && (invitationUnknown || Date.now() >= invitationExpires)) {
      enableDecision(); return;
    }
    if (decision === "approve" && marketRequired && marketLimits() === null) return;
    busy = true;
    disable();
    if (invitationMode) {
      markInvitationUnknown();
      status.textContent = "正在记录你的决定，请稍候。";
    }
    try { const data = await request("POST", decision); show(data.receipt); csrf = null; }
    catch (error) {
      factorCode.value = "";
      if (error.factor === "two_factor_code_invalid") {
        if (invitationMode) { clearInvitationUnknown(); approve.hidden = false; reject.hidden = false; }
        status.textContent = "验证码不正确或已使用。操作尚未执行，请输入当前验证码后重新批准。";
        approve.disabled = false; reject.disabled = false;
      } else if (error.factor === "two_factor_locked") {
        if (invitationMode) clearInvitationUnknown();
        status.textContent = "双因素验证暂时锁定，操作尚未执行。请稍后重新打开本页。";
        csrf = null;
      } else { failure(); }
    }
    finally { busy = false; }
  }
  approve.addEventListener("click", () => { void decide("approve"); });
  reject.addEventListener("click", () => { void decide("reject"); });
  async function load() {
    if (!/^\/api\/user\/auth\/assistant\/confirmations\/[a-f0-9]{64}$/.test(path)) throw new Error("invalid path");
    const data = await request("GET");
    show(data.confirmation);
    if (data.confirmation.state === "pending") {
      if (typeof data.csrf !== "string" || !/^[a-f0-9]{64}$/.test(data.csrf)) throw new Error("invalid browser nonce");
      csrf = data.csrf;
      if (invitationMode && invitationUnknown) {
        // Core's successful GET locks the original row and rotates its nonce.
        // Pending plus this new nonce resolves the old browser uncertainty.
        // It permits only another explicit decision on this same operation;
        // the page never submits or sends mail as a result of a status read.
        clearInvitationUnknown();
        approve.hidden = false; reject.hidden = false;
        status.textContent = "已查询原操作，确认仍待你决定。请核对后选择同意发送或取消邀请。";
      }
      enableDecision();
    }
  }
  confirmationRefresh.addEventListener("click", async () => {
    if (busy) return;
    busy = true; confirmationRefresh.disabled = true;
    try { await load(); } catch (_) { failure(); }
    finally { busy = false; confirmationRefresh.disabled = false; enableDecision(); }
  });
  void load().catch(failure);
})();

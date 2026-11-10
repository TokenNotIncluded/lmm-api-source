//! Responses WebSocket protocol state, independent of transport and money.
//!
//! Nothing here opens a socket or authorizes a model call. The host must bind
//! `PreparedCreate::wire_bytes` to its reservation before writing those bytes.
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use serde_json::{Value, value::RawValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use super::{ClientProtocol, Limits, RelayError, Request, Route, UpstreamProtocol};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProtocolError(pub &'static str);

type Result<T> = std::result::Result<T, ProtocolError>;

/// An entire session retires at these bounds; IDs are never evicted and then
/// accidentally admitted again. Neither bound creates an unbounded queue.
pub(crate) const MAX_SESSION_TURNS: usize = 256;
const MAX_SESSION_CONTROLS: usize = 256;
const MAX_ID_BYTES: usize = 256;

struct Object(BTreeMap<String, Box<RawValue>>);
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an unambiguous WebSocket event object")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Object, A::Error> {
                let mut keys = BTreeSet::new();
                let mut fields = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.to_ascii_lowercase()) {
                        return Err(serde::de::Error::custom("duplicate event key"));
                    }
                    fields.insert(key, map.next_value()?);
                }
                Ok(Object(fields))
            }
        }
        d.deserialize_map(ObjectVisitor)
    }
}

fn object(bytes: &[u8], limit: usize) -> Result<Object> {
    if bytes.len() > limit {
        return Err(ProtocolError("WebSocket event exceeds byte limit"));
    }
    // RawValue retains tool-schema order. This first bounded parse also enforces
    // serde_json's normal nesting limit before any raw-value sub-object parse.
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| ProtocolError("invalid WebSocket JSON or nesting"))?;
    if !value.is_object() {
        return Err(ProtocolError("WebSocket event must be an object"));
    }
    serde_json::from_slice(bytes).map_err(|_| ProtocolError("ambiguous WebSocket object"))
}

fn identity(raw: Option<&RawValue>) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let value: Value =
        serde_json::from_str(raw.get()).map_err(|_| ProtocolError("invalid identity"))?;
    let value = value
        .as_str()
        .ok_or(ProtocolError("identity must be a string"))?;
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err(ProtocolError("invalid identity"));
    }
    Ok(Some(value.to_owned()))
}

fn take_bool(fields: &mut BTreeMap<String, Box<RawValue>>, name: &str) -> Result<Option<bool>> {
    fields
        .remove(name)
        .map(|v| {
            serde_json::from_str(v.get()).map_err(|_| ProtocolError("generate must be a boolean"))
        })
        .transpose()
}

#[derive(Debug)]
pub(crate) struct Create {
    pub event_id: Option<String>,
    pub stream_id: Option<String>,
    pub previous_response_id: Option<String>,
    pub generate: Option<bool>,
    /// Existing Request parsing/policy checks run on this JSON. Stateful fields
    /// are restored only by prepare(), for the admitted Responses WS route.
    pub request_json: Vec<u8>,
}

impl Create {
    pub(crate) fn parse(bytes: &[u8], limit: usize) -> Result<Self> {
        let Object(mut outer) = object(bytes, limit)?;
        let kind = outer
            .remove("type")
            .ok_or(ProtocolError("missing event type"))?;
        if kind.get() != "\"response.create\"" {
            // Compare decoded text too, so valid escaped JSON has the same meaning.
            if serde_json::from_str::<String>(kind.get()).ok().as_deref() != Some("response.create")
            {
                return Err(ProtocolError("expected response.create"));
            }
        }
        let event_id = identity(outer.remove("event_id").as_deref())?;
        let stream_id = identity(outer.remove("stream_id").as_deref())?;
        let outer_generate = take_bool(&mut outer, "generate")?;
        let mut fields = if let Some(request) = outer.remove("request") {
            if !outer.is_empty() {
                return Err(ProtocolError("mixed wrapped and flat create fields"));
            }
            object(request.get().as_bytes(), limit)?.0
        } else {
            outer
        };
        let inner_generate = take_bool(&mut fields, "generate")?;
        if matches!((outer_generate, inner_generate), (Some(a), Some(b)) if a != b) {
            return Err(ProtocolError("conflicting generate fields"));
        }
        let generate = outer_generate.or(inner_generate);
        let previous_response_id = match fields.remove("previous_response_id") {
            Some(value) if value.get() == "null" => None,
            value => identity(value.as_deref())?,
        };
        // These are HTTP transport controls, not provider generation fields.
        // Background work cannot be accounted by this synchronous turn owner.
        if fields
            .get("background")
            .is_some_and(|v| v.get() != "false" && v.get() != "null")
        {
            return Err(ProtocolError("background Responses are not supported"));
        }
        for name in ["background", "stream", "stream_options"] {
            fields.remove(name);
        }
        let request_json =
            serde_json::to_vec(&fields).map_err(|_| ProtocolError("invalid create"))?;
        Ok(Self {
            event_id,
            stream_id,
            previous_response_id,
            generate,
            request_json,
        })
    }

    /// Serialize with the selected route's policy and commit the actual frame.
    pub(crate) fn prepare_selected(
        &self,
        request: &Request,
        route: &Route,
        limits: &Limits,
    ) -> std::result::Result<PreparedCreate, RelayError> {
        if request.client != ClientProtocol::Responses
            || route.protocol != UpstreamProtocol::Responses
            || !route.is_available()
        {
            return Err(RelayError::RouteUnavailable);
        }
        let selected_body = request.upstream_body(route)?;
        self.prepare(&selected_body, limits.request_bytes)
            .map_err(|error| RelayError::Invalid(error.0))
    }

    fn prepare(&self, selected_body: &[u8], limit: usize) -> Result<PreparedCreate> {
        let Object(mut fields) = object(selected_body, limit)?;
        for forbidden in [
            "type",
            "event_id",
            "stream_id",
            "previous_response_id",
            "generate",
        ] {
            if fields.contains_key(forbidden) {
                return Err(ProtocolError("selected body contains WS envelope fields"));
            }
        }
        for name in ["stream", "stream_options", "background"] {
            fields.remove(name);
        }
        let mut put = |key: &str, value: Value| -> Result<()> {
            fields.insert(
                key.to_owned(),
                RawValue::from_string(value.to_string())
                    .map_err(|_| ProtocolError("invalid create envelope"))?,
            );
            Ok(())
        };
        put("type", Value::String("response.create".into()))?;
        // Client event_id is local duplicate/control identity, matching the old
        // adapter which strips it from create. stream_id remains a wire field.
        if let Some(id) = &self.stream_id {
            put("stream_id", Value::String(id.clone()))?;
        }
        if let Some(id) = &self.previous_response_id {
            put("previous_response_id", Value::String(id.clone()))?;
        }
        if let Some(generate) = self.generate {
            put("generate", Value::Bool(generate))?;
        }
        let wire_bytes =
            serde_json::to_vec(&fields).map_err(|_| ProtocolError("invalid create"))?;
        if wire_bytes.len() > limit {
            return Err(ProtocolError("prepared create exceeds byte limit"));
        }
        let wire_sha256 = Sha256::digest(&wire_bytes).into();
        let mut hash = Sha256::new();
        hash.update(b"lmm-responses-ws-create-frame-v1\0");
        hash.update((wire_bytes.len() as u64).to_be_bytes());
        hash.update(&wire_bytes);
        Ok(PreparedCreate {
            wire_bytes,
            wire_sha256,
            frame_commitment: hash.finalize().into(),
        })
    }
}

#[derive(Debug)]
pub(crate) struct PreparedCreate {
    pub wire_bytes: Vec<u8>,
    /// Existing reservation-contract raw-wire digest; never a POST-body digest.
    pub wire_sha256: [u8; 32],
    /// Independent transport-domain commitment for the WS request fingerprint.
    pub frame_commitment: [u8; 32],
}

pub(crate) fn turn_id(
    actor: &[u8; 32],
    session: &[u8; 32],
    sequence: u64,
    event_id: Option<&str>,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"lmm-responses-ws-turn-v1\0");
    hash.update(actor);
    hash.update(session);
    match event_id {
        Some(id) => {
            hash.update([1]);
            hash.update((id.len() as u64).to_be_bytes());
            hash.update(id.as_bytes());
        }
        None => {
            hash.update([0]);
            hash.update(sequence.to_be_bytes());
        }
    }
    hash.finalize().into()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Attribution {
    Active,
    Foreign,
    Control,
    Idle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminalOutcome {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
}

/// An observed terminal boundary is not settlement or proof of zero usage.
/// The host decodes these exact counters with the existing provider decoder,
/// persists its private observation, and keeps absent/invalid counters held.
pub(crate) struct TerminalReceipt<'a> {
    pub outcome: TerminalOutcome,
    pub usage: Option<&'a Value>,
}

pub(crate) fn terminal(value: &Value) -> Result<Option<TerminalReceipt<'_>>> {
    let kind = value["type"]
        .as_str()
        .ok_or(ProtocolError("missing upstream event type"))?;
    let outcome = match kind {
        "response.completed" | "response.done" => match value["response"]["status"].as_str() {
            Some("completed") => TerminalOutcome::Completed,
            Some("incomplete") => TerminalOutcome::Incomplete,
            Some("failed") => TerminalOutcome::Failed,
            Some("cancelled" | "canceled") => TerminalOutcome::Cancelled,
            _ => return Err(ProtocolError("invalid terminal response status")),
        },
        "response.incomplete" => TerminalOutcome::Incomplete,
        "response.failed" | "response.error" | "error" => TerminalOutcome::Failed,
        "response.cancelled" | "response.canceled" => TerminalOutcome::Cancelled,
        _ => return Ok(None),
    };
    let usage = value["response"].get("usage").filter(|v| !v.is_null());
    if usage.is_some_and(|v| !v.is_object()) {
        return Err(ProtocolError("invalid terminal usage object"));
    }
    Ok(Some(TerminalReceipt { outcome, usage }))
}

struct Active {
    sequence: u64,
    event_id: Option<String>,
    stream_id: Option<String>,
    response_id: Option<String>,
}

/// Single owner state machine: finish() is called only after the detached
/// finalizer resolves. Merely observing a terminal does not admit another turn.
#[derive(Default)]
pub(crate) struct Correlation {
    sequence: u64,
    active: Option<Active>,
    creates: BTreeSet<String>,
    responses: BTreeSet<String>,
    controls: BTreeSet<String>,
}
impl Correlation {
    pub(crate) fn begin(&mut self, create: &Create) -> Result<u64> {
        if self.active.is_some() {
            return Err(ProtocolError("another response is active"));
        }
        if self.sequence >= MAX_SESSION_TURNS as u64 {
            return Err(ProtocolError("session turn limit"));
        }
        if let Some(id) = &create.event_id {
            if self.controls.contains(id) || !self.creates.insert(id.clone()) {
                return Err(ProtocolError("duplicate event identity"));
            }
        }
        self.sequence += 1;
        self.active = Some(Active {
            sequence: self.sequence,
            event_id: create.event_id.clone(),
            stream_id: create.stream_id.clone(),
            response_id: None,
        });
        Ok(self.sequence)
    }

    pub(crate) fn register_control(&mut self, event_id: &str) -> Result<()> {
        if self.active.is_none() {
            return Err(ProtocolError("no active response"));
        }
        if event_id.is_empty()
            || event_id.len() > MAX_ID_BYTES
            || !event_id.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(ProtocolError("invalid control identity"));
        }
        if self.controls.len() >= MAX_SESSION_CONTROLS {
            return Err(ProtocolError("session control limit"));
        }
        if self.creates.contains(event_id) || !self.controls.insert(event_id.into()) {
            return Err(ProtocolError("duplicate control identity"));
        }
        Ok(())
    }

    pub(crate) fn attribute(&mut self, value: &Value) -> Result<Attribution> {
        let kind = value["type"]
            .as_str()
            .ok_or(ProtocolError("missing upstream event type"))?;
        let Some(active) = self.active.as_mut() else {
            return Ok(Attribution::Idle);
        };
        let read_id = |value: &Value| -> Result<Option<String>> {
            if value.is_null() {
                return Ok(None);
            }
            let id = value
                .as_str()
                .ok_or(ProtocolError("invalid upstream identity"))?;
            if id.is_empty() || id.len() > MAX_ID_BYTES {
                return Err(ProtocolError("invalid upstream identity"));
            }
            Ok(Some(id.into()))
        };
        let stream = read_id(&value["stream_id"])?;
        if stream
            .as_ref()
            .zip(active.stream_id.as_ref())
            .is_some_and(|(a, b)| a != b)
        {
            return Ok(Attribution::Foreign);
        }
        let top_id = read_id(&value["response_id"])?;
        let nested_id = read_id(&value["response"]["id"])?;
        if top_id
            .as_ref()
            .zip(nested_id.as_ref())
            .is_some_and(|(a, b)| a != b)
        {
            return Err(ProtocolError("conflicting upstream response identity"));
        }
        let response_id = top_id.or(nested_id);
        if response_id.as_ref().is_some_and(|id| {
            self.responses.contains(id)
                || active
                    .response_id
                    .as_ref()
                    .is_some_and(|current| current != id)
        }) {
            return Ok(Attribution::Foreign);
        }
        if kind == "error" {
            let reference = read_id(&value["error"]["event_id"])?;
            if let Some(reference) = reference {
                if self.controls.contains(&reference) {
                    return Ok(Attribution::Control);
                }
                if active.event_id.as_ref() != Some(&reference) {
                    return Ok(Attribution::Foreign);
                }
            } else {
                // Without an explicit create reference a provider error could be
                // rejection of a cancel/control. Never borrow active usage.
                return Ok(Attribution::Control);
            }
        }
        if active.response_id.is_none() {
            if matches!(kind, "response.created" | "response.in_progress") {
                active.response_id =
                    Some(response_id.ok_or(ProtocolError("missing created response identity"))?);
            } else if kind != "error" {
                return Ok(Attribution::Foreign);
            }
        }
        Ok(Attribution::Active)
    }

    pub(crate) fn finish(&mut self, sequence: u64) -> Result<()> {
        if self
            .active
            .as_ref()
            .is_none_or(|active| active.sequence != sequence)
        {
            return Err(ProtocolError("stale turn finalizer"));
        }
        if let Some(id) = self.active.take().and_then(|active| active.response_id) {
            self.responses.insert(id);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "responses_ws_tests.rs"]
mod tests;

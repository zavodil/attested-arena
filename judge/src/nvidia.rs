//! The GPU side of a model's attestation: NVIDIA's evidence, judged by NVIDIA's
//! own attestation service (NRAS).
//!
//! NEAR AI's report carries `nvidia_payload`, a JSON text with the GPUs'
//! evidence and the nonce it was taken for. Posted unchanged to NRAS, it comes
//! back as a JWT signed by NVIDIA whose `x-nvidia-overall-att-result` is the
//! verdict. The token goes into the receipt as it is, and the browser checks
//! its signature against NVIDIA's published keys.

use crate::http;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Serialize;
use serde_json::Value;

const NRAS: &str = "https://nras.attestation.nvidia.com/v3/attest/gpu";

#[derive(Clone, Debug, Serialize)]
pub struct GpuCheck {
    pub ok: bool,
    /// The nonce the evidence was taken for, as the payload states it.
    pub nonce: Option<String>,
    /// NVIDIA's signed verdict, verbatim.
    pub token: Option<String>,
    pub error: Option<String>,
}

pub fn attest(payload: &str, expected_nonce: &str) -> GpuCheck {
    let nonce = serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("nonce").and_then(Value::as_str).map(str::to_string));
    let refused = |nonce: Option<String>, token: Option<String>, error: String| GpuCheck { ok: false, nonce, token, error: Some(error) };

    if nonce.as_deref() != Some(expected_nonce) {
        return refused(nonce, None, "the GPU evidence was not taken for this request's nonce".to_string());
    }
    let reply = match http::post(NRAS, &[("Content-Type", "application/json"), ("Accept", "application/json")], payload.as_bytes()) {
        Ok(r) => r,
        Err(e) => return refused(nonce, None, e),
    };
    if !reply.ok() {
        return refused(nonce, None, format!("NVIDIA refused the evidence: HTTP {} {}", reply.status, reply.excerpt()));
    }
    let token = match platform_token(&reply.body) {
        Some(t) => t,
        None => return refused(nonce, None, "NVIDIA's answer carries no token".to_string()),
    };
    match verdict(&token) {
        Some(true) => GpuCheck { ok: true, nonce, token: Some(token), error: None },
        Some(false) => refused(nonce, Some(token), "NVIDIA's verdict is negative".to_string()),
        None => refused(nonce, Some(token), "NVIDIA's token states no overall verdict".to_string()),
    }
}

/// The first entry of NRAS's answer: `["JWT", "<token>"]`.
fn platform_token(body: &[u8]) -> Option<String> {
    let answer: Value = serde_json::from_slice(body).ok()?;
    let first = answer.as_array()?.first()?.as_array()?;
    (first.first()?.as_str()? == "JWT").then(|| first.get(1)?.as_str().map(str::to_string)).flatten()
}

/// `x-nvidia-overall-att-result` from the token's payload. Read here for the
/// enclave's own verdict; the signature over it is the browser's to check.
fn verdict(token: &str) -> Option<bool> {
    let payload = token.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    claims.get("x-nvidia-overall-att-result")?.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(claims: &str) -> String {
        format!("eyJhbGciOiJFUzM4NCJ9.{}.sig", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn the_verdict_is_the_overall_claim_and_nothing_else() {
        assert_eq!(verdict(&token(r#"{"x-nvidia-overall-att-result":true}"#)), Some(true));
        assert_eq!(verdict(&token(r#"{"x-nvidia-overall-att-result":false}"#)), Some(false));
        assert_eq!(verdict(&token(r#"{"x-nvidia-overall-att-result":"true"}"#)), None, "a string is not a verdict");
        assert_eq!(verdict(&token(r#"{}"#)), None);
        assert_eq!(verdict("not-a-jwt"), None);
    }

    #[test]
    fn the_platform_token_is_the_first_jwt_entry() {
        assert_eq!(platform_token(br#"[["JWT","a.b.c"],{"GPU-0":"x.y.z"}]"#).as_deref(), Some("a.b.c"));
        assert_eq!(platform_token(br#"[["XYZ","a.b.c"]]"#), None);
        assert_eq!(platform_token(br#"{"JWT":"a.b.c"}"#), None);
    }

    #[test]
    fn evidence_for_another_nonce_is_refused_before_anything_is_sent() {
        let check = attest(r#"{"nonce":"aa","evidence_list":[]}"#, "bb");
        assert!(!check.ok);
        assert_eq!(check.nonce.as_deref(), Some("aa"));
        assert!(check.error.unwrap().contains("nonce"));
    }
}

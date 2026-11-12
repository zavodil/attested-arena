//! What the enclave establishes about a model's answers in one run.
//!
//! A model is attested once per run, for a nonce that is the hash of every
//! request the run sends it ([`arena_core::request::attestation_nonce`]):
//!
//! | check | holds when |
//! |---|---|
//! | `quote` | the instance's Intel TDX quote verifies against Intel's collateral |
//! | `binding` | the quote's report_data is `signing_address ‖ zero pad ‖ nonce` |
//! | `gpu` | NVIDIA's attestation service accepts the GPU evidence taken for that nonce |
//!
//! Then every answer must carry a `provider_tee` signature — EIP-191 over
//! `model:sha256(request):sha256(response)` — that recovers to an address
//! whose three checks hold. One unproven answer fails the run: the judge
//! signs no result built on an answer it cannot attribute.

use crate::eip191;
use crate::intel::{self, QuoteCheck};
use crate::nearai::{Client, Exchange, ModelAttestation, ResponseSignature, Tee};
use crate::nvidia::{self, GpuCheck};
use arena_core::sha256_hex;
use serde::Serialize;
use serde_json::Value;

/// One instance key's evidence, as the result carries it.
#[derive(Debug, Serialize)]
pub struct Instance {
    pub signing_address: String,
    /// Hex.
    pub intel_quote: String,
    pub quote: QuoteCheck,
    pub binding_ok: bool,
    pub gpu: GpuCheck,
    /// dstack's description of what the TEE runs: app id, compose hash, OS image.
    pub app: Value,
}

#[derive(Debug, Serialize)]
pub struct Attested {
    pub model: String,
    pub nonce: String,
    pub instances: Vec<Instance>,
}

/// `model`'s attestation report for `nonce`, fetched BEFORE any request is
/// sent: a model NEAR AI does not serve from its own TEE never sees a task.
pub fn reports(api: &Client, model: &str, nonce: &str) -> Result<Vec<ModelAttestation>, String> {
    match api.attestation(model, nonce)? {
        Tee::Attested(r) => Ok(r),
        Tee::NotAttested(why) => Err(format!("{model} has no NEAR-operated TEE attestation, so no task was sent to it: {why}")),
    }
}

/// Prove the keys that signed this run's answers: each one's instance in the
/// report must verify with Intel, bind the key to `nonce`, and pass NVIDIA.
pub fn prove(model: &str, nonce: &str, reports: Vec<ModelAttestation>, signers: &[String], now: u64) -> Result<Attested, String> {
    let mut instances: Vec<Instance> = Vec::new();
    for signer in signers {
        if instances.iter().any(|i| i.signing_address.eq_ignore_ascii_case(signer)) {
            continue;
        }
        let report = reports
            .iter()
            .find(|r| r.signing_address.eq_ignore_ascii_case(signer))
            .ok_or_else(|| format!("{model}'s answers are signed by {signer}, which its attestation report does not name"))?;
        let instance = check_instance(report.clone(), nonce, now);
        if !(instance.quote.ok && instance.binding_ok && instance.gpu.ok) {
            return Err(format!(
                "{model}'s key {signer} is not proven: quote {}, binding {}, gpu {}",
                instance.quote.error.as_deref().unwrap_or("ok"),
                if instance.binding_ok { "ok" } else { "does not hold" },
                instance.gpu.error.as_deref().unwrap_or("ok")
            ));
        }
        instances.push(instance);
    }
    Ok(Attested { model: model.to_string(), nonce: nonce.to_string(), instances })
}

fn check_instance(report: ModelAttestation, nonce: &str, now: u64) -> Instance {
    let quote_bytes = hex::decode(&report.intel_quote).unwrap_or_default();
    let quote = match intel::collateral_for(&quote_bytes) {
        Ok(collateral) => intel::verify(&quote_bytes, &collateral, now),
        Err(e) => QuoteCheck { ok: false, tcb_status: None, advisory_ids: Vec::new(), report_data: None, error: Some(format!("not verified: {e}")) },
    };
    let binding_ok = binding(&quote, &report, nonce);
    let gpu = nvidia::attest(&report.nvidia_payload, nonce);
    let app = serde_json::json!({
        "app_id": report.info.get("app_id"),
        "compose_hash": report.info.get("compose_hash"),
        "os_image_hash": report.info.get("os_image_hash"),
    });
    Instance { signing_address: report.signing_address.to_ascii_lowercase(), intel_quote: report.intel_quote, quote, binding_ok, gpu, app }
}

/// report_data = the signing address (20 bytes) ‖ 12 zero bytes ‖ the nonce
/// (32 bytes), as NEAR AI documents it.
pub fn binding(quote: &QuoteCheck, report: &ModelAttestation, nonce: &str) -> bool {
    let Some(report_data) = quote.report_data.as_deref().filter(|_| quote.ok) else { return false };
    if report.request_nonce != nonce {
        return false;
    }
    let Ok(mut expected) = hex::decode(report.signing_address.trim_start_matches("0x")) else { return false };
    if expected.len() > 32 {
        return false;
    }
    expected.resize(32, 0);
    let Ok(n) = hex::decode(nonce) else { return false };
    expected.extend(n);
    hex::encode(expected) == report_data
}

/// The answer's signature: made in the model's TEE, over this model and
/// these exact bytes. Returns the signer, to be proven by [`prove`].
pub fn signer(model: &str, exchange: &Exchange, signature: &ResponseSignature) -> Result<String, String> {
    if signature.signature_kind.as_deref() != Some("provider_tee") {
        return Err(format!(
            "the signature is `{}`, not one made in the model's TEE",
            signature.signature_kind.as_deref().unwrap_or("of no stated kind")
        ));
    }
    let expected = format!("{model}:{}:{}", sha256_hex(&exchange.request), sha256_hex(&exchange.response));
    if signature.text != expected {
        return Err("the signed text is not this model and these exact request and response bytes".to_string());
    }
    eip191::recover(&signature.text, &signature.signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "0x835ab53d20294ef795906dff21ae4c045336aa26";
    const NONCE: &str = "628e157108d4706845681bdc3a8d79f6d1aac921d45fd686bade9b7d44669f4d";

    fn report(address: &str, nonce: &str) -> ModelAttestation {
        ModelAttestation {
            model_name: "m".into(),
            signing_address: address.into(),
            signing_algo: "ecdsa".into(),
            request_nonce: nonce.into(),
            intel_quote: String::new(),
            nvidia_payload: String::new(),
            info: Value::Null,
        }
    }

    fn verified(report_data: &str) -> QuoteCheck {
        QuoteCheck { ok: true, tcb_status: Some("UpToDate".into()), advisory_ids: vec![], report_data: Some(report_data.into()), error: None }
    }

    #[test]
    fn report_data_binds_the_signing_address_and_the_nonce() {
        let report_data = format!("{}{}{NONCE}", &ADDRESS[2..], "0".repeat(24));
        assert!(binding(&verified(&report_data), &report(ADDRESS, NONCE), NONCE));
        assert!(!binding(&verified(&report_data), &report(ADDRESS, NONCE), &"00".repeat(32)), "another run's nonce");
        assert!(!binding(&verified(&report_data), &report("0x0000000000000000000000000000000000000001", NONCE), NONCE));
        let unverified = QuoteCheck { ok: false, ..verified(&report_data) };
        assert!(!binding(&unverified, &report(ADDRESS, NONCE), NONCE), "an unverified quote binds nothing");
    }

    #[test]
    fn a_gateway_signature_is_not_a_model_signature() {
        let exchange = Exchange { request: b"{}".to_vec(), response: b"{}".to_vec(), id: "x".into() };
        let signature = ResponseSignature {
            text: format!("{}:{}", sha256_hex(b"{}"), sha256_hex(b"{}")),
            signature: String::new(),
            signing_address: ADDRESS.into(),
            signing_algo: "ecdsa".into(),
            signature_kind: Some("gateway".into()),
        };
        assert!(signer("m", &exchange, &signature).unwrap_err().contains("`gateway`"));
    }
}

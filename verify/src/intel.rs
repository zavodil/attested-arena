//! Intel TDX quotes, verified here again with the collateral Intel's PCS
//! serves — the same checks the judge made inside the enclave, made outside.

use dcap_qvl::quote::Quote;
use dcap_qvl::QuoteCollateralV3;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::HashMap;

const PCS: &str = "https://api.trustedservices.intel.com";
const ROOT_CA_CRL: &str = "https://certificates.trustedservices.intel.com/IntelSGXRootCA.der";

pub struct Intel {
    http: reqwest::blocking::Client,
    cache: HashMap<String, QuoteCollateralV3>,
    /// The same collateral as dcap-qvl reads it in JSON (binary fields in
    /// hex), by `FMSPC-CA`, for the report's in-browser check.
    pub exported: HashMap<String, serde_json::Value>,
}

pub struct Verified {
    pub tcb_status: String,
    /// The 64 bytes the TEE put in the quote, hex.
    pub report_data: String,
}

#[derive(Deserialize)]
struct SignedTcbInfo<'a> {
    #[serde(rename = "tcbInfo", borrow)]
    body: &'a RawValue,
    signature: String,
}

#[derive(Deserialize)]
struct SignedQeIdentity<'a> {
    #[serde(rename = "enclaveIdentity", borrow)]
    body: &'a RawValue,
    signature: String,
}

impl Intel {
    pub fn new() -> Self {
        Intel { http: reqwest::blocking::Client::new(), cache: HashMap::new(), exported: HashMap::new() }
    }

    pub fn verify(&mut self, quote: &[u8], now: u64) -> Result<Verified, String> {
        let parsed = Quote::parse(quote).map_err(|e| format!("the quote does not parse: {e}"))?;
        let fmspc = hex::encode_upper(parsed.fmspc().map_err(|e| format!("no FMSPC: {e}"))?);
        let ca = parsed.ca().map_err(|e| format!("no PCK issuer: {e}"))?.to_string();
        let key = format!("{fmspc}-{ca}");
        if !self.cache.contains_key(&key) {
            let c = self.collateral(&fmspc, &ca)?;
            self.exported.insert(key.clone(), serde_json::json!({
                "pck_crl_issuer_chain": c.pck_crl_issuer_chain, "root_ca_crl": hex::encode(&c.root_ca_crl), "pck_crl": hex::encode(&c.pck_crl),
                "tcb_info_issuer_chain": c.tcb_info_issuer_chain, "tcb_info": c.tcb_info, "tcb_info_signature": hex::encode(&c.tcb_info_signature),
                "qe_identity_issuer_chain": c.qe_identity_issuer_chain, "qe_identity": c.qe_identity, "qe_identity_signature": hex::encode(&c.qe_identity_signature),
            }));
            self.cache.insert(key.clone(), c);
        }
        let report = dcap_qvl::verify::rustcrypto::verify(quote, &self.cache[&key], now).map_err(|e| format!("{e:#}"))?;
        let td = report.report.as_td10().ok_or("the quote is not a TD report")?;
        Ok(Verified { tcb_status: report.status, report_data: hex::encode(td.report_data) })
    }

    fn get(&self, url: &str) -> Result<reqwest::blocking::Response, String> {
        let r = self.http.get(url).send().map_err(|e| format!("{url}: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("{url}: HTTP {}", r.status()));
        }
        Ok(r)
    }

    fn collateral(&self, fmspc: &str, ca: &str) -> Result<QuoteCollateralV3, String> {
        let header = |r: &reqwest::blocking::Response, names: &[&str]| -> Result<String, String> {
            let v = names
                .iter()
                .find_map(|n| r.headers().get(*n))
                .ok_or_else(|| format!("Intel's answer has no {} header", names[0]))?;
            percent_decode(v.to_str().map_err(|_| "a header is not text")?)
        };
        let crl = self.get(&format!("{PCS}/sgx/certification/v4/pckcrl?ca={ca}&encoding=der"))?;
        let pck_crl_issuer_chain = header(&crl, &["SGX-PCK-CRL-Issuer-Chain"])?;
        let pck_crl = crl.bytes().map_err(|e| e.to_string())?.to_vec();
        let tcb = self.get(&format!("{PCS}/tdx/certification/v4/tcb?fmspc={fmspc}"))?;
        let tcb_info_issuer_chain = header(&tcb, &["TCB-Info-Issuer-Chain", "SGX-TCB-Info-Issuer-Chain"])?;
        let tcb_body = tcb.bytes().map_err(|e| e.to_string())?;
        let qe = self.get(&format!("{PCS}/tdx/certification/v4/qe/identity?update=standard"))?;
        let qe_identity_issuer_chain = header(&qe, &["SGX-Enclave-Identity-Issuer-Chain"])?;
        let qe_body = qe.bytes().map_err(|e| e.to_string())?;
        let root_ca_crl = self.get(ROOT_CA_CRL)?.bytes().map_err(|e| e.to_string())?.to_vec();
        let tcb: SignedTcbInfo = serde_json::from_slice(&tcb_body).map_err(|e| format!("TCB info: {e}"))?;
        let qe: SignedQeIdentity = serde_json::from_slice(&qe_body).map_err(|e| format!("QE identity: {e}"))?;
        Ok(QuoteCollateralV3 {
            pck_crl_issuer_chain,
            root_ca_crl,
            pck_crl,
            tcb_info_issuer_chain,
            tcb_info: tcb.body.get().to_string(),
            tcb_info_signature: hex::decode(tcb.signature).map_err(|_| "TCB signature is not hex")?,
            qe_identity_issuer_chain,
            qe_identity: qe.body.get().to_string(),
            qe_identity_signature: hex::decode(qe.signature).map_err(|_| "QE signature is not hex")?,
            pck_certificate_chain: None,
        })
    }
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = value.get(i + 1..i + 3).ok_or("a percent escape is cut short")?;
            out.push(u8::from_str_radix(pair, 16).map_err(|_| "a bad percent escape")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "a header is not UTF-8".to_string())
}

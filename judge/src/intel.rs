//! Intel TDX quotes: the collateral a quote is verified against, and the
//! verification itself.
//!
//! The collateral comes from Intel's own PCS. Every piece of it is signed by
//! Intel up to the root embedded in dcap-qvl, so where it was fetched from
//! does not matter: a changed byte fails verification. The enclave verifies
//! with it, and hands the same collateral to the browser, which verifies again
//! with dcap-qvl compiled to WebAssembly — Intel's PCS does not answer a
//! browser (no CORS), the enclave does.

use crate::http;
use dcap_qvl::quote::Quote;
use dcap_qvl::QuoteCollateralV3;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

const PCS: &str = "https://api.trustedservices.intel.com";
const ROOT_CA_CRL: &str = "https://certificates.trustedservices.intel.com/IntelSGXRootCA.der";

/// The collateral as dcap-qvl reads it in JSON: certificate chains as PEM,
/// Intel's signed documents as served, binary fields in hex.
#[derive(Clone, Debug, Serialize)]
pub struct Collateral {
    pub pck_crl_issuer_chain: String,
    pub root_ca_crl: String,
    pub pck_crl: String,
    pub tcb_info_issuer_chain: String,
    pub tcb_info: String,
    pub tcb_info_signature: String,
    pub qe_identity_issuer_chain: String,
    pub qe_identity: String,
    pub qe_identity_signature: String,
}

/// What verifying a quote established.
#[derive(Clone, Debug, Serialize)]
pub struct QuoteCheck {
    pub ok: bool,
    /// Intel's TCB status verbatim; only `UpToDate` is fully current.
    pub tcb_status: Option<String>,
    pub advisory_ids: Vec<String>,
    /// The 64 bytes the TEE put in the quote, in hex.
    pub report_data: Option<String>,
    pub error: Option<String>,
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

/// Fetch the collateral for this quote's platform (its FMSPC) and its PCK
/// issuer (processor or platform CA).
pub fn collateral_for(quote: &[u8]) -> Result<Collateral, String> {
    let parsed = Quote::parse(quote).map_err(|e| format!("the quote does not parse: {e}"))?;
    let fmspc = hex::encode_upper(parsed.fmspc().map_err(|e| format!("the quote names no FMSPC: {e}"))?);
    let ca = parsed.ca().map_err(|e| format!("the quote names no PCK issuer: {e}"))?;

    let crl = fetch(&format!("{PCS}/sgx/certification/v4/pckcrl?ca={ca}&encoding=der"))?;
    let tcb = fetch(&format!("{PCS}/tdx/certification/v4/tcb?fmspc={fmspc}"))?;
    let qe = fetch(&format!("{PCS}/tdx/certification/v4/qe/identity?update=standard"))?;
    let root_crl = fetch(ROOT_CA_CRL)?;

    let tcb_info: SignedTcbInfo =
        serde_json::from_slice(&tcb.body).map_err(|e| format!("Intel's TCB info does not read: {e}"))?;
    let qe_identity: SignedQeIdentity =
        serde_json::from_slice(&qe.body).map_err(|e| format!("Intel's QE identity does not read: {e}"))?;

    Ok(Collateral {
        pck_crl_issuer_chain: chain(&crl, &["SGX-PCK-CRL-Issuer-Chain"])?,
        root_ca_crl: hex::encode(&root_crl.body),
        pck_crl: hex::encode(&crl.body),
        tcb_info_issuer_chain: chain(&tcb, &["TCB-Info-Issuer-Chain", "SGX-TCB-Info-Issuer-Chain"])?,
        tcb_info: tcb_info.body.get().to_string(),
        tcb_info_signature: tcb_info.signature.to_ascii_lowercase(),
        qe_identity_issuer_chain: chain(&qe, &["SGX-Enclave-Identity-Issuer-Chain"])?,
        qe_identity: qe_identity.body.get().to_string(),
        qe_identity_signature: qe_identity.signature.to_ascii_lowercase(),
    })
}

/// Verify `quote` against `collateral` as of `now` (unix seconds).
pub fn verify(quote: &[u8], collateral: &Collateral, now: u64) -> QuoteCheck {
    let refused = |error: String| QuoteCheck { ok: false, tcb_status: None, advisory_ids: Vec::new(), report_data: None, error: Some(error) };
    let collateral = match collateral.to_dcap() {
        Ok(c) => c,
        Err(e) => return refused(e),
    };
    match dcap_qvl::verify::rustcrypto::verify(quote, &collateral, now) {
        Err(e) => refused(format!("{e:#}")),
        Ok(report) => {
            let report_data = report.report.as_td10().map(|td| hex::encode(td.report_data));
            QuoteCheck { ok: true, tcb_status: Some(report.status), advisory_ids: report.advisory_ids, report_data, error: None }
        }
    }
}

impl Collateral {
    fn to_dcap(&self) -> Result<QuoteCollateralV3, String> {
        let bytes = |name: &str, value: &str| hex::decode(value).map_err(|_| format!("collateral `{name}` is not hex"));
        Ok(QuoteCollateralV3 {
            pck_crl_issuer_chain: self.pck_crl_issuer_chain.clone(),
            root_ca_crl: bytes("root_ca_crl", &self.root_ca_crl)?,
            pck_crl: bytes("pck_crl", &self.pck_crl)?,
            tcb_info_issuer_chain: self.tcb_info_issuer_chain.clone(),
            tcb_info: self.tcb_info.clone(),
            tcb_info_signature: bytes("tcb_info_signature", &self.tcb_info_signature)?,
            qe_identity_issuer_chain: self.qe_identity_issuer_chain.clone(),
            qe_identity: self.qe_identity.clone(),
            qe_identity_signature: bytes("qe_identity_signature", &self.qe_identity_signature)?,
            pck_certificate_chain: None,
        })
    }
}

fn fetch(url: &str) -> Result<http::Reply, String> {
    let reply = http::get(url, &[])?;
    if !reply.ok() {
        return Err(format!("Intel refused {url}: HTTP {} {}", reply.status, reply.excerpt()));
    }
    Ok(reply)
}

/// The issuer chain Intel puts in a header, percent-decoded.
fn chain(reply: &http::Reply, names: &[&str]) -> Result<String, String> {
    let value = names
        .iter()
        .find_map(|name| reply.header(name))
        .ok_or_else(|| format!("Intel's answer carries no {} header", names[0]))?;
    http::percent_decode(value)
}

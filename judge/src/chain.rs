//! What the judge reads from NEAR itself, through the worker's RPC.

use serde_json::{json, Value};

/// The arena must be locked: no full-access key on its account, so nobody —
/// the operator included — can deploy other code to it. Then the inputs it
/// builds for the judge are the published contract's, and the season's rules
/// cannot change while it runs.
pub fn assert_locked(account: &str) -> Result<(), String> {
    let params = json!({ "request_type": "view_access_key_list", "finality": "final", "account_id": account });
    let (answer, error) = outlayer::raw::rpc::raw("query", &params.to_string());
    if !error.is_empty() {
        return Err(format!("the keys of {account} could not be read: {error}"));
    }
    let answer: Value = serde_json::from_str(&answer).map_err(|e| format!("the chain's answer about {account} does not read: {e}"))?;
    let result = answer.get("result").unwrap_or(&answer);
    if let Some(e) = result.get("error") {
        return Err(format!("the keys of {account} could not be read: {e}"));
    }
    let keys = result.get("keys").and_then(Value::as_array).ok_or_else(|| format!("the chain listed no keys for {account}"))?;
    let full = keys.iter().filter(|k| k.pointer("/access_key/permission").and_then(Value::as_str) == Some("FullAccess")).count();
    if full > 0 {
        return Err(format!(
            "the arena {account} still has {full} full-access key(s): its code could be replaced, so the judge does not act for it"
        ));
    }
    Ok(())
}

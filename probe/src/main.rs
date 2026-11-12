//! An experiment that shows why the judge's pool stays sealed: a build other
//! than the judge declares the very same key — path `pool`, `bind: "wasm"`,
//! `caller: "signer"` — and tries to open one of a season's ciphertexts,
//! run by the season's own operator, with the exact `aad` the judge uses.
//!
//! OutLayer's keystore derives a `bind: "wasm"` key from the running build's
//! sha256, so this build gets a different key and the ciphertext does not
//! open. The same holds for any later build of the judge itself.
//!
//! Input: `{"arena": "...", "leaf": "<hex>", "ciphertext": "<base64>"}`.
//! Output: `{"opened": bool, "error": "..."}` — never the plaintext.

use base64::Engine;
use serde::Deserialize;

#[cfg(target_family = "wasm")]
#[used]
#[link_section = "outlayer.manifest"]
static OUTLAYER_MANIFEST: [u8; include_bytes!("../manifest.json").len()] = *include_bytes!("../manifest.json");

#[derive(Deserialize)]
struct Input {
    arena: String,
    leaf: String,
    ciphertext: String,
}

fn main() {
    let result = (|| -> Result<(), String> {
        let input: Input = serde_json::from_slice(&outlayer::env::input()).map_err(|e| e.to_string())?;
        let ct = base64::engine::general_purpose::STANDARD.decode(&input.ciphertext).map_err(|e| e.to_string())?;
        let aad = format!("attested-arena/task:{}:{}", input.arena, input.leaf);
        outlayer::encryption_keys::decrypt("pool", None, &ct, aad.as_bytes()).map(|_| ()).map_err(|e| e.to_string())
    })();
    let out = match result {
        Ok(()) => serde_json::json!({ "opened": true }),
        Err(e) => serde_json::json!({ "opened": false, "error": e }),
    };
    outlayer::env::output_json(&out).expect("the result serialises");
}

//! The model's response signature: an Ethereum personal-sign (EIP-191)
//! signature over `model:sha256(request):sha256(response)`, made inside the
//! model-serving TEE with the key its attestation report names.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest, Keccak256};

/// The address that signed `text`, as `0x` + 40 lowercase hex digits.
pub fn recover(text: &str, signature_hex: &str) -> Result<String, String> {
    let raw = hex::decode(signature_hex.trim_start_matches("0x")).map_err(|_| "the signature is not hex".to_string())?;
    if raw.len() != 65 {
        return Err(format!("an EIP-191 signature is 65 bytes; this one is {}", raw.len()));
    }
    let signature = Signature::from_slice(&raw[..64]).map_err(|_| "the signature's r and s do not parse".to_string())?;
    let v = match raw[64] {
        v @ (27 | 28) => v - 27,
        v @ (0 | 1) => v,
        v => return Err(format!("the signature's recovery byte {v} is neither 0/1 nor 27/28")),
    };
    let recovery = RecoveryId::from_byte(v).ok_or("the signature's recovery byte is out of range")?;
    let key = VerifyingKey::recover_from_prehash(&personal_hash(text), &signature, recovery)
        .map_err(|_| "no key recovers from this signature".to_string())?;
    Ok(address(&key))
}

fn personal_hash(text: &str) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(format!("\x19Ethereum Signed Message:\n{}", text.len()).as_bytes());
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

fn address(key: &VerifyingKey) -> String {
    let point = key.to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash[12..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    fn sign(key: &SigningKey, text: &str) -> String {
        let (signature, recovery) = key.sign_prehash_recoverable(&personal_hash(text)).unwrap();
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery.to_byte() + 27);
        hex::encode(raw)
    }

    #[test]
    fn the_signer_is_recovered_and_a_changed_text_names_someone_else() {
        let key = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let signer = address(key.verifying_key());
        let text = "Qwen/Qwen3.8-27B:aa:bb";
        assert_eq!(recover(text, &sign(&key, text)).unwrap(), signer);
        assert_eq!(recover(text, &format!("0x{}", sign(&key, text))).unwrap(), signer);
        assert_ne!(recover("Qwen/Qwen3.8-27B:aa:bc", &sign(&key, text)).unwrap(), signer);
    }

    /// The well-known vector: private key 0x…01 is address
    /// 0x7e5f4552091a69125d5dfcb7b8c2659029395bdf.
    #[test]
    fn the_address_of_the_key_one_is_the_known_one() {
        let mut one = [0u8; 32];
        one[31] = 1;
        let key = SigningKey::from_slice(&one).unwrap();
        assert_eq!(address(key.verifying_key()), "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
    }

    #[test]
    fn malformed_signatures_say_what_is_wrong() {
        assert!(recover("x", "zz").unwrap_err().contains("not hex"));
        assert!(recover("x", "abcd").unwrap_err().contains("65 bytes"));
        let mut raw = vec![1u8; 64];
        raw.push(9);
        assert!(recover("x", &hex::encode(raw)).unwrap_err().contains("recovery byte 9"));
    }
}

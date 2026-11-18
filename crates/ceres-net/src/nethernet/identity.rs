//! NetherNet server identity: the answer SDP carries `a=identity:` with ES384 JWS tokens binding
//! our DTLS fingerprints to a P-384 key. Port of altay's ServerIdentity / SdpFingerprints.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use p384::ecdsa::{Signature, SigningKey, signature::Signer};
use p384::pkcs8::EncodePublicKey;

const TOKEN_LIFETIME: u64 = 60;

pub struct ServerIdentity {
    key: SigningKey,
    public_key_b64: String,
    domain: String,
}

impl ServerIdentity {
    pub fn generate(domain: &str) -> Self {
        let key = loop {
            let bytes: [u8; 48] = rand::random();
            if let Ok(k) = SigningKey::from_slice(&bytes) {
                break k;
            }
        };
        Self::from_key(key, domain)
    }

    /// Loads the 48-byte P-384 signing key from `path`, or generates and saves one when missing.
    pub fn load_or_generate(path: &std::path::Path, domain: &str) -> Self {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(key) = SigningKey::from_slice(&bytes) {
                return Self::from_key(key, domain);
            }
        }
        let id = Self::generate(domain);
        let _ = std::fs::write(path, id.key.to_bytes().as_slice());
        id
    }

    fn from_key(key: SigningKey, domain: &str) -> Self {
        let der = key.verifying_key().to_public_key_der().expect("encode public key");
        ServerIdentity { public_key_b64: STANDARD.encode(der.as_bytes()), key, domain: domain.to_string() }
    }

    fn sign_compact(&self, header: &str, payload: &str, detached: bool) -> String {
        let h = URL_SAFE_NO_PAD.encode(header);
        let p = URL_SAFE_NO_PAD.encode(payload);
        let sig: Signature = self.key.sign(format!("{h}.{p}").as_bytes());
        let s = URL_SAFE_NO_PAD.encode(sig.to_bytes());
        format!("{h}.{}.{s}", if detached { "" } else { &p })
    }

    /// Value for `a=identity:` (base64), or None if the SDP has no fingerprint.
    pub fn identity_attribute(&self, sdp: &str) -> Option<String> {
        let parts: Vec<String> = sdp
            .lines()
            .filter_map(|l| l.trim().strip_prefix("a=fingerprint:"))
            .filter_map(|v| v.split_once(' '))
            .map(|(alg, digest)| {
                format!(r#"{{"algorithm":{},"digest":{}}}"#, json_str(alg.trim()), json_str(digest.trim()))
            })
            .collect();
        if parts.is_empty() {
            return None;
        }
        let payload = format!(r#"{{"fingerprint":[{}]}}"#, parts.join(","));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let token = self.sign_compact(
            &format!(r#"{{"alg":"ES384","x5u":"{}"}}"#, self.public_key_b64),
            &format!(r#"{{"cpk":"{}","exp":{},"iat":{now}}}"#, self.public_key_b64, now + TOKEN_LIFETIME),
            false,
        );
        let fingerprints = self.sign_compact(r#"{"alg":"ES384"}"#, &payload, true);
        let inner = format!(r#"{{"fingerprints":{},"token":{}}}"#, json_str(&fingerprints), json_str(&token));
        let identity = format!(
            r#"{{"assertion":{},"idp":{{"domain":{},"protocol":"default"}}}}"#,
            json_str(&inner),
            json_str(&self.domain)
        );
        Some(STANDARD.encode(identity))
    }
}

/// json_encode() of a string (PHP escapes "/" by default).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap().replace('/', "\\/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_signs_sdp_fingerprint() {
        let id = ServerIdentity::generate("self");
        let sdp = "v=0\r\na=fingerprint:sha-256 AA:BB:CC:DD\r\n";
        let attr = id.identity_attribute(sdp).expect("identity attribute");
        let decoded = STANDARD.decode(attr).unwrap();
        let val: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(val["idp"]["domain"], "self");
        assert!(val["assertion"].as_str().unwrap().contains("fingerprints"));
    }
}

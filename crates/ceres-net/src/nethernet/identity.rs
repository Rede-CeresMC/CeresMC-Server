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

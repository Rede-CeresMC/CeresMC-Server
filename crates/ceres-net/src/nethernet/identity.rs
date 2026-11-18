//! NetherNet identities. Both sides put `a=identity:` in their SDP: a token holding a P-384 key
//! (`cpk`) and a JWS made with that key over the DTLS fingerprints. Ours is self-signed; the
//! client's token is issued by Minecraft's auth service (the same kind as the login token).
//! Checking the client's fingerprint signature ties its key to this very connection, so the login
//! can later be required to carry that same key (a login token copied from elsewhere is useless).
//! Port of altay's ServerIdentity / ClientIdentityAssertion / SdpFingerprints.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use p384::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier};
use p384::pkcs8::{DecodePublicKey, EncodePublicKey};

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
        let payload = fingerprint_payload(sdp)?;
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

/// The JSON the fingerprint JWS signs: every `a=fingerprint` of the SDP, in order.
fn fingerprint_payload(sdp: &str) -> Option<String> {
    let parts: Vec<String> = sdp
        .lines()
        .filter_map(|l| l.trim().strip_prefix("a=fingerprint:"))
        .filter_map(|v| v.split_once(' '))
        .map(|(alg, digest)| format!(r#"{{"algorithm":{},"digest":{}}}"#, json_str(alg.trim()), json_str(digest.trim())))
        .collect();
    (!parts.is_empty()).then(|| format!(r#"{{"fingerprint":[{}]}}"#, parts.join(",")))
}

fn b64url_json(part: &str) -> Result<serde_json::Value, String> {
    let bytes = URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).map_err(|_| "bad base64 in identity")?;
    serde_json::from_slice(&bytes).map_err(|_| "bad JSON in identity".into())
}

fn verify_es384(signing_input: &str, sig_b64: &str, key: &VerifyingKey) -> bool {
    let Ok(sig) = URL_SAFE_NO_PAD.decode(sig_b64) else { return false };
    let Ok(sig) = Signature::from_slice(&sig) else { return false };
    key.verify(signing_input.as_bytes(), &sig).is_ok()
}

/// Checks the client's `a=identity` in an SDP offer. Returns the client key (`cpk`, base64 DER)
/// the offer is bound to, None when the offer has no identity, or why the identity is invalid.
pub fn client_identity(sdp: &str, now: u64) -> Result<Option<String>, String> {
    let Some(attr) = sdp.lines().find_map(|l| l.trim().strip_prefix("a=identity:")) else { return Ok(None) };
    let outer: serde_json::Value =
        serde_json::from_slice(&STANDARD.decode(attr.trim()).map_err(|_| "identity is not base64")?)
            .map_err(|_| "identity is not JSON")?;
    if outer["idp"]["protocol"].as_str() != Some("default") || outer["idp"]["domain"].as_str().is_none_or(str::is_empty)
    {
        return Err("unsupported identity provider".into());
    }
    let assertion: serde_json::Value =
        serde_json::from_str(outer["assertion"].as_str().ok_or("identity without assertion")?)
            .map_err(|_| "identity assertion is not JSON")?;
    let (Some(fingerprints), Some(token)) = (assertion["fingerprints"].as_str(), assertion["token"].as_str()) else {
        return Err("identity assertion without fingerprints or token".into());
    };
    let token_parts: Vec<&str> = token.split('.').collect();
    let fp_parts: Vec<&str> = fingerprints.split('.').collect();
    if token_parts.len() != 3 || fp_parts.len() != 3 {
        return Err("malformed identity tokens".into());
    }
    let alg = b64url_json(token_parts[0])?["alg"].as_str().unwrap_or("").to_string();
    if alg != "RS256" && alg != "ES384" {
        return Err(format!("identity token uses {alg}"));
    }
    let claims = b64url_json(token_parts[1])?;
    let cpk = claims["cpk"].as_str().ok_or("identity token without cpk")?.to_string();
    if claims["exp"].as_u64().is_some_and(|exp| exp + 60 < now) {
        return Err("identity token expired".into());
    }
    let key = VerifyingKey::from_public_key_der(&STANDARD.decode(&cpk).map_err(|_| "bad cpk encoding")?)
        .map_err(|_| "bad cpk key")?;
    // a self-signed token must be signed by the key it names; an issued one (RS256) is checked
    // against Mojang's keys by the login, which must then carry this same cpk
    if alg == "ES384" && !verify_es384(&format!("{}.{}", token_parts[0], token_parts[1]), token_parts[2], &key) {
        return Err("identity token is not signed by its key".into());
    }
    if b64url_json(fp_parts[0])?["alg"].as_str() != Some("ES384") {
        return Err("unsupported fingerprint signature".into());
    }
    let payload = fingerprint_payload(sdp).ok_or("offer without DTLS fingerprint")?;
    let input = format!("{}.{}", fp_parts[0], URL_SAFE_NO_PAD.encode(payload));
    if !verify_es384(&input, fp_parts[2], &key) {
        return Err("DTLS fingerprints are not signed by the identity key".into());
    }
    Ok(Some(cpk))
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

    #[test]
    fn client_identity_is_checked() {
        let id = ServerIdentity::generate("self");
        let sdp = "v=0\r\na=fingerprint:sha-256 AA:BB:CC:DD\r\n";
        let attr = id.identity_attribute(sdp).unwrap();
        let signed = format!("{sdp}a=identity:{attr}\r\n");
        assert_eq!(client_identity(&signed, 0).unwrap().as_deref(), Some(id.public_key_b64.as_str()));
        // another DTLS certificate than the one signed
        let swapped = signed.replace("AA:BB", "EE:FF");
        assert!(client_identity(&swapped, 0).is_err());
        // someone else's identity pasted into this offer
        let other = ServerIdentity::generate("self").identity_attribute("v=0\r\na=fingerprint:sha-256 11:22\r\n").unwrap();
        assert!(client_identity(&format!("{sdp}a=identity:{other}\r\n"), 0).is_err());
        assert!(client_identity(sdp, 0).unwrap().is_none());
        // expired
        assert!(client_identity(&signed, u64::MAX / 2).is_err());
    }

    /// A real client offer: CERES_OFFER=path cargo test -p ceres-net real_offer -- --ignored
    #[test]
    #[ignore]
    fn real_offer() {
        let sdp = std::fs::read_to_string(std::env::var("CERES_OFFER").unwrap()).unwrap();
        assert!(client_identity(&sdp, 0).unwrap().is_some());
        assert!(client_identity(&sdp.replacen("a=fingerprint:sha-256 ", "a=fingerprint:sha-256 00", 1), 0).is_err());
    }
}

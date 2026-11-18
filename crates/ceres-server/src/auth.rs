//! Xbox Live login verification (OpenID tokens, Bedrock 1.21.90+).
//! Port of Altay's FetchAuthKeysTask / ProcessOpenIdLoginTask / AuthJwtHelper:
//!
//! - the login carries a JWT signed by Mojang (RS256). Its keys are published at
//!   discovery -> `{auth service}/.well-known/openid-configuration` -> `jwks_uri`.
//! - the token must have the right issuer, the multiplayer audience and be within its validity.
//!   It holds the XUID (`xid`), the gamertag (`xname`) and the client's public key (`cpk`).
//! - the client data JWT must be signed (ES384) with that `cpk`.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DISCOVERY_URL: &str = "https://client.discovery.minecraft-services.net/api/v1.0/discovery/MinecraftPE/builds/";
const AUTH_SERVICE_FALLBACK: &str = "https://authorization.franchise.minecraft-services.net";
const AUDIENCE: &str = "api://auth-minecraft-services/multiplayer";
/// clock difference tolerated between the server and Mojang
const LEEWAY_SECS: i64 = 60;

pub struct MojangKeys {
    issuer: String,
    keys: HashMap<String, rsa::PublicKey>,
}

pub type SharedKeys = Arc<RwLock<Option<Arc<MojangKeys>>>>;

/// A login that passed verification.
pub struct Verified {
    pub xuid: String,
    pub gamertag: String,
    /// the client's key (base64 DER) the token vouches for
    pub cpk: String,
}

fn http_get(url: &str) -> Result<String, String> {
    let mut resp = ureq::get(url).call().map_err(|e| format!("{url}: {e}"))?;
    resp.body_mut().read_to_string().map_err(|e| format!("{url}: {e}"))
}

fn http_json(url: &str) -> Result<Value, String> {
    serde_json::from_str(&http_get(url)?).map_err(|e| format!("{url}: {e}"))
}

fn fetch_keys(version: &str) -> Result<MojangKeys, String> {
    let service = http_json(&format!("{DISCOVERY_URL}{version}"))
        .ok()
        .and_then(|d| d["result"]["serviceEnvironments"]["auth"]["prod"]["serviceUri"].as_str().map(str::to_string))
        .unwrap_or_else(|| AUTH_SERVICE_FALLBACK.to_string());
    let config = http_json(&format!("{}/.well-known/openid-configuration", service.trim_end_matches('/')))?;
    let issuer = config["issuer"].as_str().ok_or("openid configuration without issuer")?.to_string();
    let jwks_uri = config["jwks_uri"].as_str().ok_or("openid configuration without jwks_uri")?;
    let jwks = http_json(jwks_uri)?;
    let mut keys = HashMap::new();
    for k in jwks["keys"].as_array().ok_or("jwks without keys")? {
        if k["kty"].as_str() != Some("RSA") {
            continue;
        }
        let (Some(kid), Some(n), Some(e)) = (k["kid"].as_str(), k["n"].as_str(), k["e"].as_str()) else { continue };
        let (Ok(n), Ok(e)) = (URL_SAFE_NO_PAD.decode(n), URL_SAFE_NO_PAD.decode(e)) else { continue };
        let key =
            rsa::PublicKey::new(rsa::BoxedUint::from_be_slice_vartime(&n), rsa::BoxedUint::from_be_slice_vartime(&e));
        if let Ok(key) = key {
            keys.insert(kid.to_string(), key);
        }
    }
    if keys.is_empty() {
        return Err("no usable RSA keys in the JWKS".into());
    }
    Ok(MojangKeys { issuer, keys })
}

/// Fetches Mojang's keys now and every hour (every minute while it keeps failing).
pub fn spawn_key_fetcher(version: &'static str) -> SharedKeys {
    let shared: SharedKeys = Arc::new(RwLock::new(None));
    let out = shared.clone();
    std::thread::Builder::new()
        .name("xbox-auth-keys".into())
        .spawn(move || {
            loop {
                let wait = match fetch_keys(version) {
                    Ok(k) => {
                        println!("[Auth] Xbox Live keys loaded ({} keys, issuer {})", k.keys.len(), k.issuer);
                        *out.write().unwrap() = Some(Arc::new(k));
                        3600
                    }
                    Err(e) => {
                        eprintln!("[Auth] could not fetch Xbox Live keys: {e}");
                        60
                    }
                };
                std::thread::sleep(Duration::from_secs(wait));
            }
        })
        .ok();
    shared
}

fn split_jwt(jwt: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = jwt.split('.');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(b), Some(s), None) => Ok((h, b, s)),
        _ => Err("malformed JWT".into()),
    }
}

fn b64url_json(part: &str) -> Result<Value, String> {
    let bytes = URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).map_err(|e| format!("JWT base64: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("JWT json: {e}"))
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn verify_rs256(jwt: &str, keys: &MojangKeys) -> Result<Value, String> {
    use rsa::signature::Verifier;
    let (h, b, s) = split_jwt(jwt)?;
    let header = b64url_json(h)?;
    if header["alg"].as_str() != Some("RS256") {
        return Err(format!("unexpected token algorithm {}", header["alg"]));
    }
    let kid = header["kid"].as_str().ok_or("token without kid")?;
    let key = keys.keys.get(kid).ok_or_else(|| format!("unknown signing key {kid}"))?;
    let sig = URL_SAFE_NO_PAD.decode(s).map_err(|_| "bad token signature encoding")?;
    let sig = rsa::pkcs1v15::Signature::try_from(&sig[..]).map_err(|_| "bad token signature")?;
    rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha256>::new(key.clone())
        .verify(format!("{h}.{b}").as_bytes(), &sig)
        .map_err(|_| "invalid token signature")?;
    let claims = b64url_json(b)?;
    if claims["iss"].as_str() != Some(keys.issuer.as_str()) {
        return Err("invalid token issuer".into());
    }
    let aud_ok = match &claims["aud"] {
        Value::String(a) => a == AUDIENCE,
        Value::Array(list) => list.iter().any(|a| a.as_str() == Some(AUDIENCE)),
        _ => false,
    };
    if !aud_ok {
        return Err("invalid token audience".into());
    }
    let t = now();
    if claims["exp"].as_i64().is_some_and(|exp| exp + LEEWAY_SECS < t) {
        return Err("token expired".into());
    }
    if claims["nbf"].as_i64().is_some_and(|nbf| nbf - LEEWAY_SECS > t) {
        return Err("token not valid yet".into());
    }
    Ok(claims)
}

/// The client data JWT must be signed (ES384) by the key the Mojang token vouches for.
fn verify_client_data(jwt: &str, cpk_der: &[u8]) -> Result<(), String> {
    use p384::ecdsa::signature::Verifier;
    use p384::pkcs8::DecodePublicKey;
    let (h, b, s) = split_jwt(jwt)?;
    let header = b64url_json(h)?;
    let x5u = header["x5u"].as_str().ok_or("client data without x5u")?;
    if STANDARD.decode(x5u).map_err(|_| "bad x5u")? != cpk_der {
        return Err("client data signed by another key".into());
    }
    let key = p384::ecdsa::VerifyingKey::from_public_key_der(cpk_der).map_err(|_| "bad client public key")?;
    let sig = URL_SAFE_NO_PAD.decode(s).map_err(|_| "bad client data signature encoding")?;
    let sig = p384::ecdsa::Signature::from_slice(&sig).map_err(|_| "bad client data signature")?;
    key.verify(format!("{h}.{b}").as_bytes(), &sig).map_err(|_| "invalid client data signature".into())
}

/// Verifies a login. `auth_json` is the login's authentication info, `client_data` the client data JWT.
pub fn verify_login(auth_json: &[u8], client_data: &[u8], keys: Option<&MojangKeys>) -> Result<Verified, String> {
    let keys = keys.ok_or("Xbox Live keys are not loaded yet")?;
    let auth: Value = serde_json::from_slice(auth_json).map_err(|_| "bad authentication info")?;
    let token = auth["Token"].as_str().filter(|t| !t.is_empty()).ok_or("login without an Xbox Live token")?;
    let claims = verify_rs256(token, keys)?;
    let cpk = claims["cpk"].as_str().ok_or("token without client key")?;
    let cpk_der = STANDARD.decode(cpk).map_err(|_| "bad client key encoding")?;
    let client_data = std::str::from_utf8(client_data).map_err(|_| "bad client data")?;
    verify_client_data(client_data, &cpk_der)?;
    let xuid = claims["xid"].as_str().filter(|x| !x.is_empty()).ok_or("token without XUID")?.to_string();
    let gamertag = claims["xname"].as_str().filter(|x| !x.is_empty()).ok_or("token without gamertag")?.to_string();
    Ok(Verified { xuid, gamertag, cpk: cpk.to_string() })
}

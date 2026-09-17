use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::types::JobError;

type HmacSha256 = Hmac<Sha256>;

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn sign_body(secret: &str, timestamp: u64, body: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC_SHA256 accepts any key length");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_framegif_signature(
    secret: &str,
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    body: &[u8],
    now: u64,
) -> Result<(), JobError> {
    let timestamp: u64 = timestamp_header
        .ok_or_else(|| JobError::msg("missing x-timestamp"))?
        .parse()
        .map_err(|_| JobError::msg("invalid x-timestamp"))?;
    if now.abs_diff(timestamp) > 300 {
        return Err(JobError::msg("expired signature"));
    }
    let expected = sign_body(secret, timestamp, body);
    let given = signature_header.ok_or_else(|| JobError::msg("missing x-signature"))?;
    if !constant_time_eq(expected.as_bytes(), given.as_bytes()) {
        return Err(JobError::msg("bad signature"));
    }
    Ok(())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (l, r)| acc | (l ^ r)) == 0
}

pub fn verify_qstash_signature(
    signing_key: &str,
    token: &str,
    body: &[u8],
    url: &str,
    now: u64,
) -> Result<(), JobError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(JobError::msg("invalid qstash jwt"));
    }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let mut mac = HmacSha256::new_from_slice(signing_key.as_bytes())
        .map_err(|_| JobError::msg("qstash key"))?;
    mac.update(signing_input.as_bytes());
    let expected = mac.finalize().into_bytes();
    let given = b64url_decode(parts[2])?;
    if !constant_time_eq(&expected, &given) {
        return Err(JobError::msg("qstash signature mismatch"));
    }
    let payload_bytes = b64url_decode(parts[1])?;
    let payload: serde_json::Value =
        serde_json::from_slice(&payload_bytes).map_err(|_| JobError::msg("qstash payload"))?;
    if payload.get("iss").and_then(serde_json::Value::as_str) != Some("Upstash") {
        return Err(JobError::msg("qstash issuer"));
    }
    if let Some(sub) = payload.get("sub").and_then(serde_json::Value::as_str)
        && !sub.is_empty()
        && sub != url
    {
        return Err(JobError::msg("qstash subject"));
    }
    if let Some(exp) = payload.get("exp").and_then(serde_json::Value::as_u64)
        && now > exp
    {
        return Err(JobError::msg("qstash expired"));
    }
    if let Some(nbf) = payload.get("nbf").and_then(serde_json::Value::as_u64)
        && now < nbf
    {
        return Err(JobError::msg("qstash not yet valid"));
    }
    if let Some(body_hash) = payload.get("body").and_then(serde_json::Value::as_str) {
        let digest = Sha256::digest(body);
        let encoded =
            base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, digest);
        if encoded != body_hash {
            return Err(JobError::msg("qstash body hash"));
        }
    }
    Ok(())
}

fn b64url_decode(input: &str) -> Result<Vec<u8>, JobError> {
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, input)
        .map_err(|_| JobError::msg("b64url"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_roundtrip() {
        let secret = "test-secret";
        let body = b"{\"ok\":true}";
        let ts = 1_700_000_000;
        let sig = sign_body(secret, ts, body);
        verify_framegif_signature(secret, Some(&ts.to_string()), Some(&sig), body, ts).unwrap();
        assert!(
            verify_framegif_signature(secret, Some(&ts.to_string()), Some("deadbeef"), body, ts)
                .is_err()
        );
    }
}

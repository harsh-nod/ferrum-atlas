use crate::{Error, Result};
use atlas_model::*;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Cursor {
    pub version: u32,
    pub snapshot: SnapshotId,
    pub context: ContextId,
    pub scope: String,
    pub query: String,
    pub last_name: String,
    pub last_id: String,
    pub expires: u64,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn encode(cursor: &Cursor, secret: &[u8; 32]) -> Result<String> {
    let body = serde_json::to_vec(cursor)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts 32-byte key");
    mac.update(&body);
    Ok(format!(
        "{}.{}",
        hex(&body),
        hex(&mac.finalize().into_bytes())
    ))
}

pub(crate) fn decode(encoded: &str, secret: &[u8; 32]) -> Result<Cursor> {
    if encoded.len() > 16_384 {
        return Err(Error::InvalidCursor);
    }
    let (body, signature) = encoded.split_once('.').ok_or(Error::InvalidCursor)?;
    let body = unhex(body)?;
    let signature = unhex(signature)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts 32-byte key");
    mac.update(&body);
    mac.verify_slice(&signature)
        .map_err(|_| Error::InvalidCursor)?;
    let cursor: Cursor = serde_json::from_slice(&body).map_err(|_| Error::InvalidCursor)?;
    if cursor.version != 1 {
        return Err(Error::InvalidCursor);
    }
    if cursor.expires <= now() {
        return Err(Error::ExpiredCursor);
    }
    Ok(cursor)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unhex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(Error::InvalidCursor);
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            Ok(digit(pair[0]).ok_or(Error::InvalidCursor)? * 16
                + digit(pair[1]).ok_or(Error::InvalidCursor)?)
        })
        .collect()
}

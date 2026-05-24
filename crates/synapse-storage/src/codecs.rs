use serde::{Serialize, de::DeserializeOwned};

use crate::{StorageError, StorageResult};

// ADR-0001 / RUSTSEC-2025-0141 prohibit binary persisted codecs here; storage
// payloads stay JSON so source-of-truth bytes remain inspectable.

/// Encodes a typed storage payload as JSON bytes.
///
/// # Errors
///
/// Returns [`StorageError::EncodeJson`] when serde cannot serialize `value`.
#[tracing::instrument(skip_all, fields(storage_codec = "json", type_name = std::any::type_name::<T>()))]
pub fn encode_json<T>(value: &T) -> StorageResult<Vec<u8>>
where
    T: Serialize,
{
    serde_json::to_vec(value).map_err(|source| StorageError::EncodeJson {
        type_name: std::any::type_name::<T>(),
        source,
    })
}

/// Decodes JSON bytes into a typed storage payload.
///
/// # Errors
///
/// Returns [`StorageError::DecodeJson`] when `bytes` are not valid for `T`.
#[tracing::instrument(skip_all, fields(storage_codec = "json", type_name = std::any::type_name::<T>(), bytes = bytes.len()))]
pub fn decode_json<T>(bytes: &[u8]) -> StorageResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|source| StorageError::DecodeJson {
        type_name: std::any::type_name::<T>(),
        source,
    })
}

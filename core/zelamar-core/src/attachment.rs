//! Files too big for a message: sealed in chunks on the device, stored as
//! opaque blobs by the service (ADR-0016, `protocol/attachments.md`).
//!
//! A file gets a fresh key and a random blob id. Each 1 MiB chunk is sealed
//! with ChaCha20-Poly1305; the chunk's index and whether it is the last are
//! bound into it, so chunks can be neither reordered nor dropped from the
//! end. The key travels inside the (MLS-encrypted) message.

use chacha20poly1305::ChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use openmls_rust_crypto::RustCrypto;
use openmls_traits::random::OpenMlsRand;
use zeroize::Zeroizing;

use crate::device::Device;
use crate::error::{CoreError, Result};
use crate::service::{self, Request};

/// Plaintext bytes per chunk.
pub const CHUNK: usize = 1 << 20;

/// Most chunks in one file.
pub const MAX_CHUNKS: u32 = 25;

/// Largest file, in bytes.
pub const MAX_FILE: u64 = MAX_CHUNKS as u64 * CHUNK as u64;

/// Bytes a sealed chunk adds to its plaintext.
pub const TAG: usize = 16;

const CONTEXT: &[u8] = b"zelamar/file/v0";

/// A random identifier of a stored blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobId(pub [u8; 16]);

/// A file sealed for upload: where it goes, the key to open it, and the
/// sealed chunks in order.
pub struct SealedFile {
    /// Where the chunks are stored.
    pub blob: BlobId,
    /// Opens the chunks; travels only inside the message.
    pub key: Zeroizing<[u8; 32]>,
    /// The plaintext size.
    pub size: u64,
    /// The sealed chunks, in order.
    pub chunks: Vec<Vec<u8>>,
}

/// How many chunks a file of `size` bytes takes (an empty file takes one).
#[must_use]
pub fn chunk_count(size: u64) -> u32 {
    u32::try_from(size.div_ceil(CHUNK as u64).max(1)).unwrap_or(u32::MAX)
}

fn cipher(key: &[u8; 32]) -> Result<ChaCha20Poly1305> {
    ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| CoreError::crypto("initializing the file cipher", format!("{e:?}")))
}

fn nonce(index: u32) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[8..].copy_from_slice(&index.to_be_bytes());
    nonce
}

fn aad(index: u32, last: bool) -> Vec<u8> {
    [CONTEXT, &index.to_be_bytes(), &[u8::from(last)]].concat()
}

/// Seals `plaintext` under a fresh key, for a fresh blob.
///
/// # Errors
///
/// [`CoreError::TooLarge`] above [`MAX_FILE`], or a crypto error.
pub fn seal_file(plaintext: &[u8]) -> Result<SealedFile> {
    let size = plaintext.len() as u64;
    if size > MAX_FILE {
        return Err(CoreError::TooLarge {
            what: "file",
            size: plaintext.len(),
            limit: usize::try_from(MAX_FILE).unwrap_or(usize::MAX),
        });
    }
    let random = RustCrypto::default();
    let key: Zeroizing<[u8; 32]> = Zeroizing::new(
        random
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a file key", e))?,
    );
    let blob = BlobId(
        random
            .random_array()
            .map_err(|e| CoreError::crypto("drawing a blob id", e))?,
    );
    let cipher = cipher(&key)?;
    let count = chunk_count(size);
    let pieces: Vec<&[u8]> = if plaintext.is_empty() {
        vec![&[]]
    } else {
        plaintext.chunks(CHUNK).collect()
    };
    let chunks = pieces
        .iter()
        .zip(0u32..)
        .map(|(piece, index)| {
            cipher
                .encrypt(
                    &nonce(index).into(),
                    Payload {
                        msg: piece,
                        aad: &aad(index, index + 1 == count),
                    },
                )
                .map_err(|e| CoreError::crypto("sealing a file chunk", format!("{e:?}")))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SealedFile {
        blob,
        key,
        size,
        chunks,
    })
}

/// Opens all of a file's sealed chunks, in order, and checks its size.
///
/// # Errors
///
/// [`CoreError::Unauthenticated`] for any chunk that is not exactly what
/// was sealed in that place, [`CoreError::Malformed`] for a wrong count or
/// size.
pub fn open_file(key: &[u8; 32], size: u64, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
    let count = chunk_count(size);
    if chunks.len() != count as usize {
        return Err(CoreError::malformed("file", "wrong number of chunks"));
    }
    let cipher = cipher(key)?;
    let mut plaintext = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    for (chunk, index) in chunks.iter().zip(0u32..) {
        let piece = cipher
            .decrypt(
                &nonce(index).into(),
                Payload {
                    msg: chunk,
                    aad: &aad(index, index + 1 == count),
                },
            )
            .map_err(|_| CoreError::Unauthenticated("a file chunk did not open"))?;
        plaintext.extend_from_slice(&piece);
    }
    if plaintext.len() as u64 != size {
        return Err(CoreError::malformed("file", "wrong size"));
    }
    Ok(plaintext)
}

/// The path of chunk `index` of `blob`.
fn chunk_path(blob: BlobId, index: u32) -> String {
    format!("/v0/blobs/{}/{index}", service::b64(&blob.0))
}

/// Fetches chunk `index` of `blob`.
#[must_use]
pub fn download(blob: BlobId, index: u32) -> Request {
    Request {
        method: "GET",
        path: chunk_path(blob, index),
        headers: Vec::new(),
        body: Vec::new(),
    }
}

/// Reads the answer to [`download`]: the sealed chunk.
///
/// # Errors
///
/// [`CoreError::Service`]: `unknown_blob` once it expired.
pub fn downloaded(status: u16, body: &[u8]) -> Result<Vec<u8>> {
    if status == 200 {
        return Ok(body.to_vec());
    }
    service::expect_success(status, body, &[200]).map(|()| Vec::new())
}

/// Reads the answer to an upload.
///
/// # Errors
///
/// [`CoreError::Service`] (`unauthorized`, `blob_taken`, `rate_limited`, ...).
pub fn uploaded(status: u16, body: &[u8]) -> Result<()> {
    service::expect_success(status, body, &[200, 201])
}

impl Device {
    /// Stores chunk `index` of `file` on the service, on behalf of this
    /// device: it proves its mailbox, as when reading it.
    ///
    /// # Errors
    ///
    /// [`CoreError::Malformed`] for an index past the last chunk.
    pub fn upload(&self, file: &SealedFile, index: u32) -> Result<Request> {
        let chunk = file
            .chunks
            .get(index as usize)
            .ok_or(CoreError::malformed("file", "no such chunk"))?;
        let count = u32::try_from(file.chunks.len()).unwrap_or(u32::MAX);
        Ok(Request {
            method: "PUT",
            path: chunk_path(file.blob, index),
            headers: vec![
                ("content-type", "application/octet-stream".to_owned()),
                (
                    "authorization",
                    format!("Bearer {}", service::b64(self.mailbox_secret())),
                ),
                ("zelamar-mailbox", service::b64(&self.mailbox().0)),
                ("zelamar-chunks", count.to_string()),
            ],
            body: chunk.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_round_trip_in_chunks() {
        for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK + 7] {
            let plaintext: Vec<u8> = (0..size)
                .map(|i| u8::try_from(i % 251).expect("byte"))
                .collect();
            let sealed = seal_file(&plaintext).expect("seal");
            assert_eq!(sealed.size, size as u64);
            assert_eq!(sealed.chunks.len(), chunk_count(size as u64) as usize);
            assert!(sealed.chunks.iter().all(|c| c.len() <= CHUNK + TAG));
            assert_eq!(
                open_file(&sealed.key, sealed.size, &sealed.chunks).expect("open"),
                plaintext
            );
        }
    }

    #[test]
    fn chunks_cannot_be_swapped_dropped_or_changed() {
        let plaintext = vec![7u8; 2 * CHUNK + 10];
        let sealed = seal_file(&plaintext).expect("seal");
        let key = &sealed.key;

        let mut swapped = sealed.chunks.clone();
        swapped.swap(0, 1);
        assert!(open_file(key, sealed.size, &swapped).is_err());

        // Dropping the last chunk and claiming a shorter file fails: the
        // new last chunk was not sealed as last.
        let cut = &sealed.chunks[..2];
        assert!(open_file(key, 2 * CHUNK as u64, cut).is_err());

        let mut changed = sealed.chunks.clone();
        changed[2][0] ^= 1;
        assert!(matches!(
            open_file(key, sealed.size, &changed),
            Err(CoreError::Unauthenticated(_))
        ));
        assert!(open_file(&[0; 32], sealed.size, &sealed.chunks).is_err());
        assert!(open_file(key, sealed.size + 1, &sealed.chunks).is_err());
    }

    #[test]
    fn every_file_gets_its_own_key_and_blob() {
        let a = seal_file(b"same").expect("seal");
        let b = seal_file(b"same").expect("seal");
        assert_ne!(a.blob, b.blob);
        assert_ne!(*a.key, *b.key);
        assert_ne!(a.chunks, b.chunks);
    }

    #[test]
    fn files_have_a_size_limit() {
        assert_eq!(chunk_count(MAX_FILE), MAX_CHUNKS);
        let too_big = vec![0u8; usize::try_from(MAX_FILE).expect("size") + 1];
        assert!(matches!(
            seal_file(&too_big),
            Err(CoreError::TooLarge { .. })
        ));
    }
}

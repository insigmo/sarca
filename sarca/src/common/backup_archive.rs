//! Container format for database backups (`.sarcabak`).
//!
//! A backup has to survive being copied to another machine and restored there,
//! so the file is self-describing: a fixed header says which format version
//! wrote it and whether the payload is encrypted, and everything after the
//! header is a gzip stream of the raw `SQLite` snapshot.
//!
//! ```text
//! magic  "SARCABK1"   8 bytes
//! version             1 byte   (FORMAT_VERSION)
//! flags               1 byte   (bit0: encrypted)
//! -- encrypted only --
//! salt               16 bytes  (PBKDF2)
//! nonce prefix        8 bytes  (nonce = prefix || u32be frame counter)
//! frames             [is_last u8][len u32be][ciphertext len bytes]…
//! ```
//!
//! Encryption is AES-256-GCM over ~1 MiB frames with a PBKDF2-HMAC-SHA256 key,
//! so a large metadata database never has to be held in memory at once. Each
//! frame authenticates its own index and the "this is the last frame" flag,
//! which is what makes truncating, reordering or dropping frames a decryption
//! failure rather than a silently shorter database.

use std::io::{self, Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};

/// File signature; the trailing digit is the container generation.
pub const MAGIC: &[u8; 8] = b"SARCABK1";
/// Bumped only for a change an older reader could not parse.
pub const FORMAT_VERSION: u8 = 1;

const FLAG_ENCRYPTED: u8 = 0b0000_0001;

const SALT_LEN: usize = 16;
/// Nonce = this random prefix plus a big-endian frame counter, so no two frames
/// of one archive ever share a nonce under the same key.
const NONCE_PREFIX_LEN: usize = NONCE_LEN - 4;
const TAG_LEN: usize = 16;
/// Plaintext bytes per encrypted frame.
const FRAME_PLAINTEXT_LEN: usize = 1024 * 1024;
/// Refuse an absurd frame length rather than allocating whatever the file claims.
const MAX_FRAME_LEN: usize = FRAME_PLAINTEXT_LEN * 4 + TAG_LEN;

/// PBKDF2 work factor. Deliberately expensive: the archive is a full copy of
/// every credential-adjacent row in the database, and whoever steals a copy
/// gets unlimited offline guesses at the password protecting it.
const PBKDF2_ITERATIONS: u32 = 600_000;

/// What a header says about the archive that follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveHeader {
    pub version: u8,
    pub encrypted: bool,
}

/// Read and validate the fixed header, leaving `src` at the payload.
pub fn read_header<R: Read>(src: &mut R) -> io::Result<ArchiveHeader> {
    let mut magic = [0u8; MAGIC.len()];
    src.read_exact(&mut magic).map_err(|_| invalid("this file is not a Sarca backup"))?;
    if &magic != MAGIC {
        return Err(invalid("this file is not a Sarca backup"));
    }

    let mut meta = [0u8; 2];
    src.read_exact(&mut meta).map_err(|_| invalid("backup file is truncated"))?;
    let [version, flags] = meta;
    if version != FORMAT_VERSION {
        return Err(invalid(format!(
            "backup format v{version} was written by a newer Sarca; upgrade before restoring"
        )));
    }

    Ok(ArchiveHeader {
        version,
        encrypted: flags & FLAG_ENCRYPTED != 0,
    })
}

/// Write `src` into `dst` as a `.sarcabak` archive, encrypting when a password
/// is given.
///
/// Blocking (gzip + PBKDF2 + AEAD); call from `spawn_blocking`.
pub fn encode<R: Read, W: Write>(
    mut src: R,
    dst: &mut W,
    password: Option<&str>,
) -> io::Result<()> {
    let flags = if password.is_some() { FLAG_ENCRYPTED } else { 0 };
    dst.write_all(MAGIC)?;
    dst.write_all(&[FORMAT_VERSION, flags])?;

    let Some(password) = password else {
        let mut gz = GzEncoder::new(dst, Compression::default());
        io::copy(&mut src, &mut gz)?;
        gz.finish()?;
        return Ok(());
    };

    let rng = SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    let mut nonce_prefix = [0u8; NONCE_PREFIX_LEN];
    rng.fill(&mut salt).map_err(|_| other("system randomness unavailable"))?;
    rng.fill(&mut nonce_prefix).map_err(|_| other("system randomness unavailable"))?;

    dst.write_all(&salt)?;
    dst.write_all(&nonce_prefix)?;

    let key = derive_key(password, &salt)?;
    let mut frames = FrameWriter::new(dst, key, nonce_prefix);
    let mut gz = GzEncoder::new(&mut frames, Compression::default());
    io::copy(&mut src, &mut gz)?;
    gz.finish()?;
    frames.finish()
}

/// Decode a `.sarcabak` archive back into the raw `SQLite` bytes.
///
/// A wrong password surfaces as [`io::ErrorKind::InvalidData`] — GCM cannot tell
/// "wrong key" from "tampered ciphertext", and neither can we. A password-less
/// attempt at an encrypted archive is [`io::ErrorKind::PermissionDenied`], which
/// is what lets the caller ask for one instead of reporting corruption.
///
/// Blocking; call from `spawn_blocking`.
pub fn decode<R: Read, W: Write>(
    mut src: R,
    dst: &mut W,
    password: Option<&str>,
) -> io::Result<()> {
    let header = read_header(&mut src)?;

    if !header.encrypted {
        let mut gz = GzDecoder::new(src);
        io::copy(&mut gz, dst)?;
        return Ok(());
    }

    let password = password.ok_or_else(|| {
        io::Error::new(io::ErrorKind::PermissionDenied, "this backup is password-protected")
    })?;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_prefix = [0u8; NONCE_PREFIX_LEN];
    src.read_exact(&mut salt).map_err(|_| invalid("backup file is truncated"))?;
    src.read_exact(&mut nonce_prefix).map_err(|_| invalid("backup file is truncated"))?;

    let key = derive_key(password, &salt)?;
    let mut gz = GzDecoder::new(FrameReader::new(src, key, nonce_prefix));
    io::copy(&mut gz, dst)?;
    Ok(())
}

fn derive_key(password: &str, salt: &[u8]) -> io::Result<LessSafeKey> {
    let iterations =
        std::num::NonZeroU32::new(PBKDF2_ITERATIONS).expect("PBKDF2_ITERATIONS is non-zero");
    let mut key_bytes = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut key_bytes,
    );
    let unbound = UnboundKey::new(&AES_256_GCM, &key_bytes)
        .map_err(|_| other("failed to derive backup key"))?;
    Ok(LessSafeKey::new(unbound))
}

/// Per-frame associated data. Binding the counter and the end-of-stream flag is
/// what stops an archive from being truncated or its frames shuffled without the
/// tag check noticing.
fn frame_aad(counter: u32, is_last: bool) -> [u8; 14] {
    let mut aad = [0u8; 14];
    aad[..8].copy_from_slice(MAGIC);
    aad[8] = FORMAT_VERSION;
    aad[9..13].copy_from_slice(&counter.to_be_bytes());
    aad[13] = u8::from(is_last);
    aad
}

fn nonce_for(prefix: [u8; NONCE_PREFIX_LEN], counter: u32) -> Nonce {
    let mut bytes = [0u8; NONCE_LEN];
    bytes[..NONCE_PREFIX_LEN].copy_from_slice(&prefix);
    bytes[NONCE_PREFIX_LEN..].copy_from_slice(&counter.to_be_bytes());
    Nonce::assume_unique_for_key(bytes)
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn other(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// Buffers plaintext and emits sealed frames; `finish` writes the final frame.
struct FrameWriter<'w, W: Write> {
    inner: &'w mut W,
    key: LessSafeKey,
    nonce_prefix: [u8; NONCE_PREFIX_LEN],
    counter: u32,
    buf: Vec<u8>,
}

impl<'w, W: Write> FrameWriter<'w, W> {
    fn new(inner: &'w mut W, key: LessSafeKey, nonce_prefix: [u8; NONCE_PREFIX_LEN]) -> Self {
        Self {
            inner,
            key,
            nonce_prefix,
            counter: 0,
            buf: Vec::with_capacity(FRAME_PLAINTEXT_LEN + TAG_LEN),
        }
    }

    fn seal_buffered(&mut self, is_last: bool) -> io::Result<()> {
        let aad = frame_aad(self.counter, is_last);
        let nonce = nonce_for(self.nonce_prefix, self.counter);
        self.key
            .seal_in_place_append_tag(nonce, Aad::from(aad), &mut self.buf)
            .map_err(|_| other("failed to encrypt backup"))?;

        let len = u32::try_from(self.buf.len()).map_err(|_| other("backup frame too large"))?;
        self.inner.write_all(&[u8::from(is_last)])?;
        self.inner.write_all(&len.to_be_bytes())?;
        self.inner.write_all(&self.buf)?;

        self.buf.clear();
        self.counter = self.counter.checked_add(1).ok_or_else(|| other("backup too large"))?;
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        self.seal_buffered(true)?;
        self.inner.flush()
    }
}

impl<W: Write> Write for FrameWriter<'_, W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let take = data.len().min(FRAME_PLAINTEXT_LEN - self.buf.len());
        self.buf.extend_from_slice(&data[..take]);
        if self.buf.len() == FRAME_PLAINTEXT_LEN {
            self.seal_buffered(false)?;
        }
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Deliberately not sealing a partial frame: the frame boundary is part
        // of the format, and gzip flushes far more often than once per megabyte.
        self.inner.flush()
    }
}

/// Reads sealed frames back into a plaintext byte stream.
struct FrameReader<R: Read> {
    inner: R,
    key: LessSafeKey,
    nonce_prefix: [u8; NONCE_PREFIX_LEN],
    counter: u32,
    buf: Vec<u8>,
    /// Read cursor into `buf`.
    pos: usize,
    /// The frame flagged as last has been decrypted; nothing more may follow.
    done: bool,
}

impl<R: Read> FrameReader<R> {
    fn new(inner: R, key: LessSafeKey, nonce_prefix: [u8; NONCE_PREFIX_LEN]) -> Self {
        Self {
            inner,
            key,
            nonce_prefix,
            counter: 0,
            buf: Vec::new(),
            pos: 0,
            done: false,
        }
    }

    /// Pull and decrypt one frame into `buf`. `Ok(false)` means the stream ended
    /// cleanly on its flagged last frame.
    fn next_frame(&mut self) -> io::Result<bool> {
        if self.done {
            return Ok(false);
        }

        let mut head = [0u8; 5];
        self.inner
            .read_exact(&mut head)
            .map_err(|_| invalid("backup file is truncated or corrupted"))?;
        let is_last = head[0] == 1;
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if !(TAG_LEN..=MAX_FRAME_LEN).contains(&len) {
            return Err(invalid("backup file is corrupted"));
        }

        self.buf.resize(len, 0);
        self.inner
            .read_exact(&mut self.buf)
            .map_err(|_| invalid("backup file is truncated or corrupted"))?;

        let aad = frame_aad(self.counter, is_last);
        let nonce = nonce_for(self.nonce_prefix, self.counter);
        let plaintext_len = self
            .key
            .open_in_place(nonce, Aad::from(aad), &mut self.buf)
            .map_err(|_| invalid("wrong password, or the backup file is corrupted"))?
            .len();

        self.buf.truncate(plaintext_len);
        self.pos = 0;
        self.counter = self.counter.checked_add(1).ok_or_else(|| invalid("backup too large"))?;
        self.done = is_last;
        Ok(true)
    }
}

impl<R: Read> Read for FrameReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.buf.len() {
            if !self.next_frame()? {
                return Ok(0);
            }
        }
        let take = out.len().min(self.buf.len() - self.pos);
        out[..take].copy_from_slice(&self.buf[self.pos..self.pos + take]);
        self.pos += take;
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(payload: &[u8], password: Option<&str>) -> Vec<u8> {
        let mut archive = Vec::new();
        encode(payload, &mut archive, password).unwrap();
        archive
    }

    fn round_trip(payload: &[u8], password: Option<&str>) -> Vec<u8> {
        let archive = encoded(payload, password);
        let mut restored = Vec::new();
        decode(archive.as_slice(), &mut restored, password).unwrap();
        restored
    }

    #[test]
    fn plain_round_trip() {
        let payload = b"SQLite format 3\0some metadata".repeat(100);
        assert_eq!(round_trip(&payload, None), payload);
    }

    #[test]
    fn encrypted_round_trip_across_many_frames() {
        // Larger than one frame, so the counter / last-frame bookkeeping runs.
        let payload: Vec<u8> =
            (0..FRAME_PLAINTEXT_LEN * 2 + 7919).map(|i| (i % 251) as u8).collect();
        assert_eq!(round_trip(&payload, Some("correct horse")), payload);
    }

    #[test]
    fn empty_payload_round_trips() {
        assert_eq!(round_trip(b"", Some("pw")), Vec::<u8>::new());
        assert_eq!(round_trip(b"", None), Vec::<u8>::new());
    }

    #[test]
    fn header_reports_encryption() {
        let plain = encoded(b"x", None);
        let secret = encoded(b"x", Some("pw"));
        assert!(!read_header(&mut plain.as_slice()).unwrap().encrypted);
        assert!(read_header(&mut secret.as_slice()).unwrap().encrypted);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let archive = encoded(b"payload", Some("right"));
        let err = decode(archive.as_slice(), &mut Vec::new(), Some("wrong")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn missing_password_is_reported_separately_from_corruption() {
        let archive = encoded(b"payload", Some("pw"));
        let err = decode(archive.as_slice(), &mut Vec::new(), None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    // The whole point of authenticating the frame counter and last-frame flag:
    // lopping bytes off the end must fail loudly instead of restoring a
    // database that silently lost its tail.
    #[test]
    fn truncated_archive_is_rejected() {
        let payload: Vec<u8> = (0..FRAME_PLAINTEXT_LEN * 2).map(|i| (i % 251) as u8).collect();
        let mut archive = encoded(&payload, Some("pw"));
        archive.truncate(archive.len() / 2);
        assert!(decode(archive.as_slice(), &mut Vec::new(), Some("pw")).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut archive = encoded(b"payload that matters", Some("pw"));
        let last = archive.len() - 1;
        archive[last] ^= 0xFF;
        assert!(decode(archive.as_slice(), &mut Vec::new(), Some("pw")).is_err());
    }

    #[test]
    fn foreign_file_is_not_mistaken_for_a_backup() {
        let err = read_header(&mut b"PK\x03\x04not-a-backup".as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn an_encrypted_archive_does_not_leak_its_payload() {
        let payload = b"telegram-bot-token-1234567890".repeat(50);
        let archive = encoded(&payload, Some("pw"));
        assert!(
            !archive.windows(16).any(|w| w == &payload[..16]),
            "plaintext must not appear in the archive"
        );
    }
}

//! Shared backup envelope, independent of destination transport.
//! V2 frames use RustCrypto STREAM (AES-256-GCM), authenticating the header,
//! frame lengths, ordering and final frame. Unencrypted copies use SHA-256 for
//! accidental-corruption detection only; they provide no confidentiality.
use super::{MAX_DECODED, Snapshot};
use aes_gcm::{
    Aes256Gcm,
    aead::{
        Payload,
        stream::{DecryptorBE32, EncryptorBE32},
    },
};
use anyhow::Context;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Write};
use zeroize::{Zeroize, Zeroizing};

pub const MAGIC: &[u8; 8] = b"SHEPBK02";
const HEADER: usize = 40;
const CHUNK: usize = 64 * 1024;
const LAST: u32 = 1 << 31;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    Zstd,
    None,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protection {
    #[default]
    Passphrase,
    None,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub compression: Compression,
    pub protection: Protection,
}
impl Options {
    pub fn encrypted(self) -> bool {
        self.protection == Protection::Passphrase
    }
    pub fn compressed(self) -> bool {
        self.compression == Compression::Zstd
    }
}

pub fn recognized_prefix(bytes: &[u8]) -> bool {
    bytes.starts_with(super::MAGIC) || bytes.starts_with(MAGIC)
}

pub fn options(bytes: &[u8]) -> anyhow::Result<Options> {
    if bytes.starts_with(super::MAGIC) {
        return Ok(Options::default());
    }
    anyhow::ensure!(
        bytes.len() >= HEADER && bytes.starts_with(MAGIC),
        "This is not a supported Shep backup."
    );
    anyhow::ensure!(
        bytes[8] & !3 == 0 && bytes[9..16] == [0; 7] && bytes[39] == 0,
        "Unsupported or damaged backup format options."
    );
    Ok(Options {
        compression: if bytes[8] & 1 != 0 {
            Compression::Zstd
        } else {
            Compression::None
        },
        protection: if bytes[8] & 2 != 0 {
            Protection::Passphrase
        } else {
            Protection::None
        },
    })
}

fn key(header: &[u8], passphrase: Option<&SecretString>) -> anyhow::Result<Zeroizing<[u8; 32]>> {
    let passphrase = passphrase.context("Enter this copy's original passphrase to continue.")?;
    let mut key = Zeroizing::new([0u8; 32]);
    // These are part of SHEPBK02, independent of future crate defaults.
    let argon = argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon2::Params::new(19 * 1024, 2, 1, Some(32)).expect("fixed V2 parameters"),
    );
    argon
        .hash_password_into(
            passphrase.expose_secret().as_bytes(),
            &header[16..32],
            key.as_mut(),
        )
        .map_err(|_| anyhow::anyhow!("Could not derive the backup encryption key."))?;
    Ok(key)
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Encoder<W> {
    sink: W,
    header: [u8; HEADER],
    cipher: Option<EncryptorBE32<Aes256Gcm>>,
    checksum: Option<Sha256>,
    pending: Zeroizing<Vec<u8>>,
}
impl<W: Write> Encoder<W> {
    fn new(
        mut sink: W,
        options: Options,
        passphrase: Option<&SecretString>,
    ) -> anyhow::Result<Self> {
        let mut header = [0; HEADER];
        header[..8].copy_from_slice(MAGIC);
        header[8] = u8::from(options.compressed()) | (u8::from(options.encrypted()) << 1);
        rand::thread_rng().fill_bytes(&mut header[16..39]);
        let cipher = if options.encrypted() {
            let passphrase = passphrase.context("Choose a backup passphrase.")?;
            anyhow::ensure!(
                passphrase.expose_secret().chars().count() >= 12,
                "Use a backup passphrase of at least 12 characters."
            );
            let key = key(&header, Some(passphrase))?;
            Some(EncryptorBE32::<Aes256Gcm>::new(
                key.as_ref().into(),
                header[32..39].into(),
            ))
        } else {
            None
        };
        let mut checksum = (!options.encrypted()).then(Sha256::new);
        if let Some(checksum) = &mut checksum {
            checksum.update(header);
        }
        sink.write_all(&header)?;
        Ok(Self {
            sink,
            header,
            cipher,
            checksum,
            pending: Zeroizing::new(Vec::with_capacity(CHUNK)),
        })
    }
    fn frame(&mut self, last: bool) -> io::Result<()> {
        let length = self.pending.len() + if self.cipher.is_some() { 16 } else { 0 };
        let prefix = (length as u32 | if last { LAST } else { 0 }).to_be_bytes();
        let mut aad = self.header.to_vec();
        aad.extend(prefix);
        let data = if let Some(cipher) = &mut self.cipher {
            let payload = Payload {
                msg: &self.pending,
                aad: &aad,
            };
            if last {
                self.cipher
                    .take()
                    .expect("cipher exists")
                    .encrypt_last(payload)
            } else {
                cipher.encrypt_next(payload)
            }
            .map_err(|_| invalid("Backup encryption failed or exceeded its frame counter."))?
        } else {
            self.pending.to_vec()
        };
        self.sink.write_all(&prefix)?;
        self.sink.write_all(&data)?;
        if let Some(checksum) = &mut self.checksum {
            checksum.update(prefix);
            checksum.update(&data);
        }
        self.pending.as_mut_slice().zeroize();
        self.pending.clear();
        Ok(())
    }
    fn finish(mut self) -> io::Result<W> {
        self.frame(true)?;
        if let Some(checksum) = self.checksum.take() {
            self.sink.write_all(&checksum.finalize())?;
        }
        Ok(self.sink)
    }
}
impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let accepted = bytes.len().min(CHUNK - self.pending.len());
        self.pending.extend_from_slice(&bytes[..accepted]);
        if self.pending.len() == CHUNK {
            self.frame(false)?;
        }
        Ok(accepted)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }
}

struct Decoder<R> {
    source: R,
    header: [u8; HEADER],
    cipher: Option<DecryptorBE32<Aes256Gcm>>,
    checksum: Option<Sha256>,
    chunk: Zeroizing<Vec<u8>>,
    offset: usize,
    finished: bool,
}
impl<R: Read> Decoder<R> {
    fn new(mut source: R, passphrase: Option<&SecretString>) -> anyhow::Result<(Self, Options)> {
        let mut header = [0; HEADER];
        source.read_exact(&mut header)?;
        let options = options(&header)?;
        let cipher = if options.encrypted() {
            let key = key(&header, passphrase)?;
            Some(DecryptorBE32::<Aes256Gcm>::new(
                key.as_ref().into(),
                header[32..39].into(),
            ))
        } else {
            None
        };
        let mut checksum = (!options.encrypted()).then(Sha256::new);
        if let Some(checksum) = &mut checksum {
            checksum.update(header);
        }
        Ok((
            Self {
                source,
                header,
                cipher,
                checksum,
                chunk: Zeroizing::new(Vec::new()),
                offset: 0,
                finished: false,
            },
            options,
        ))
    }
    fn frame(&mut self) -> io::Result<()> {
        let mut prefix = [0; 4];
        self.source.read_exact(&mut prefix)?;
        let encoded = u32::from_be_bytes(prefix);
        let last = encoded & LAST != 0;
        let length = (encoded & !LAST) as usize;
        let tag = if self.cipher.is_some() { 16 } else { 0 };
        if length < tag || length > CHUNK + tag || (!last && length != CHUNK + tag) {
            return Err(invalid("Invalid backup frame length."));
        }
        let mut data = Zeroizing::new(vec![0; length]);
        self.source.read_exact(&mut data)?;
        let mut aad = self.header.to_vec();
        aad.extend(prefix);
        if let Some(checksum) = &mut self.checksum {
            checksum.update(prefix);
            checksum.update(&data);
        }
        self.chunk = Zeroizing::new(if let Some(cipher) = &mut self.cipher {
            let payload = Payload {
                msg: &data,
                aad: &aad,
            };
            if last {
                self.cipher
                    .take()
                    .expect("cipher exists")
                    .decrypt_last(payload)
            } else {
                cipher.decrypt_next(payload)
            }
            .map_err(|_| invalid("Incorrect passphrase or damaged backup."))?
        } else {
            data.to_vec()
        });
        self.offset = 0;
        if last {
            if let Some(checksum) = self.checksum.take() {
                let mut expected = [0; 32];
                self.source.read_exact(&mut expected)?;
                if checksum.finalize().as_slice() != expected {
                    return Err(invalid("The unencrypted backup checksum does not match."));
                }
            }
            if self.source.read(&mut [0])? != 0 {
                return Err(invalid("Unexpected data after the backup's final frame."));
            }
            self.finished = true;
        }
        Ok(())
    }
}
impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if self.offset == self.chunk.len() && !self.finished {
            self.frame()?;
        }
        let count = output.len().min(self.chunk.len() - self.offset);
        output[..count].copy_from_slice(&self.chunk[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

pub fn encode_to<W: Write>(
    snapshot: &Snapshot,
    options: Options,
    passphrase: Option<&SecretString>,
    sink: W,
) -> anyhow::Result<W> {
    anyhow::ensure!(
        options.encrypted() || snapshot.credentials.is_empty(),
        "Account passwords require an encrypted backup."
    );
    let encoder = Encoder::new(sink, options, passphrase)?;
    let encoder = if options.compressed() {
        let mut compressed = zstd::Encoder::new(encoder, 3)?;
        serde_json::to_writer(&mut compressed, snapshot)?;
        compressed.finish()?
    } else {
        let mut encoder = encoder;
        serde_json::to_writer(&mut encoder, snapshot)?;
        encoder
    };
    Ok(encoder.finish()?)
}
pub fn encode(
    snapshot: &Snapshot,
    options: Options,
    passphrase: Option<&SecretString>,
) -> anyhow::Result<Vec<u8>> {
    encode_to(snapshot, options, passphrase, Vec::new())
}

pub fn decode(bytes: &[u8], passphrase: Option<&SecretString>) -> anyhow::Result<Snapshot> {
    if bytes.starts_with(super::MAGIC) {
        return super::decrypt_legacy(
            bytes,
            passphrase.context("Enter this copy's original passphrase to continue.")?,
        );
    }
    let (decoder, options) = Decoder::new(bytes, passphrase)?;
    let reader: Box<dyn Read + '_> = if options.compressed() {
        Box::new(zstd::Decoder::new(decoder)?)
    } else {
        Box::new(decoder)
    };
    // The surrounding snapshot/import model still has its existing size ceiling.
    // The envelope itself reads/writes bounded frames so R23 can stream later.
    let mut decoded = Zeroizing::new(Vec::new());
    reader.take(MAX_DECODED + 1).read_to_end(&mut decoded)?;
    anyhow::ensure!(
        decoded.len() as u64 <= MAX_DECODED,
        "The backup exceeds the restore size limit."
    );
    let snapshot: Snapshot = serde_json::from_slice(&decoded)?;
    anyhow::ensure!(
        options.encrypted() || snapshot.credentials.is_empty(),
        "Account passwords require an encrypted backup."
    );
    snapshot.validate()?;
    Ok(snapshot)
}
pub fn verify(bytes: &[u8], passphrase: Option<&SecretString>) -> anyhow::Result<Options> {
    if bytes.starts_with(super::MAGIC) {
        super::verify_passphrase(
            bytes,
            passphrase.context("Enter the pending copy's original passphrase to continue.")?,
        )?;
        return Ok(Options::default());
    }
    let (mut decoder, options) = Decoder::new(bytes, passphrase)?;
    io::copy(&mut decoder, &mut io::sink())?;
    Ok(options)
}

#[cfg(test)]
pub(crate) mod tests;

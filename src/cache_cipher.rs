//! Authenticated, page-based cache encryption. Key retrieval and migration are
//! owned by background workers; this module never accesses an OS credential UI.
use anyhow::{Context, ensure};
use rusqlite::{Connection, OpenFlags};
use secrecy::{ExposeSecret, SecretBox};
use std::{fmt, path::Path};
use zeroize::Zeroizing;
pub mod key_store;
pub mod migration;
pub mod ownership;

const KEY_PREFIX: &str = "shep-cache-key-v1:";

pub(crate) fn profile_connections(
    key: Option<std::sync::Arc<Key>>,
) -> shep_profile_core::history::ConnectionFactory {
    match key {
        Some(key) => shep_profile_core::history::ConnectionFactory::new(move |path| {
            key.open(path, OpenFlags::default())
                .map_err(|_| shep_profile_core::history::Error::Storage)
        }),
        None => Default::default(),
    }
}

/// Open an independent database owner with the same device-key decision. A
/// supplied key never falls back to a plaintext connection after an error.
pub(crate) fn open(key: Option<&Key>, path: &Path, flags: OpenFlags) -> anyhow::Result<Connection> {
    match key {
        Some(key) => key.open(path, flags),
        None => Ok(Connection::open_with_flags(path, flags)?),
    }
}

/// A random device-local key, never a password, settings value or debug field.
pub struct Key(SecretBox<[u8; 32]>);

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Key([REDACTED])")
    }
}

impl Key {
    pub fn generate() -> anyhow::Result<Self> {
        use rand::RngCore;
        let mut key = Box::new([0u8; 32]);
        rand::rngs::OsRng.try_fill_bytes(key.as_mut()).context(
            "Could not obtain secure randomness for the cache key. No database was encrypted.",
        )?;
        Ok(Self(SecretBox::new(key)))
    }

    pub fn decode(value: &str) -> anyhow::Result<Self> {
        let raw = value.strip_prefix(KEY_PREFIX).context("The saved cache key has an unsupported format. Keep the database and recover its original key.")?;
        ensure!(
            raw.len() == 64 && raw.bytes().all(|b| b.is_ascii_hexdigit()),
            "The saved cache key is damaged. Keep the database and recover its original key."
        );
        let mut key = Box::new([0; 32]);
        for (output, chunk) in key.iter_mut().zip(raw.as_bytes().chunks_exact(2)) {
            let digit = |b: u8| {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b.to_ascii_lowercase() - b'a' + 10
                }
            };
            *output = (digit(chunk[0]) << 4) | digit(chunk[1]);
        }
        Ok(Self(SecretBox::new(key)))
    }

    pub fn encode(&self) -> Zeroizing<String> {
        let mut value = Zeroizing::new(KEY_PREFIX.to_string());
        self.append_hex(&mut value);
        value
    }

    fn append_hex(&self, output: &mut String) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in self.0.expose_secret() {
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 15) as usize] as char);
        }
    }

    fn sql_key(&self) -> Zeroizing<String> {
        let mut value = Zeroizing::new(String::with_capacity(67));
        value.push_str("x'");
        self.append_hex(&mut value);
        value.push('\'');
        value
    }

    /// Call before the first database read. Use the C key API so a SQL trace
    /// cannot include secret key material. SQLCipher copies the key internally.
    pub fn initialize(&self, connection: &Connection) -> anyhow::Result<()> {
        self.apply(connection, c"main")?;
        // The bundled native policy enforces this for keyed main databases,
        // including after application authorizers are changed or removed.
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        let cipher: String = connection.query_row("PRAGMA cipher_version", [], |row| row.get(0))?;
        ensure!(
            !cipher.is_empty(),
            "This build does not provide cache encryption."
        );
        // This intentionally verifies the key before schema creation or repair.
        connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get::<_, i64>(0))
            .context("The cache could not be unlocked. Its key may be missing or the database may be damaged. The original database was kept.")?;
        Ok(())
    }

    pub(crate) fn apply(
        &self,
        connection: &Connection,
        schema: &std::ffi::CStr,
    ) -> anyhow::Result<()> {
        let key = self.sql_key();
        // SAFETY: the live connection owns this handle, the key buffer remains
        // valid for the synchronous call, and its length fits c_int (67 bytes).
        let result = unsafe {
            rusqlite::ffi::sqlite3_key_v2(
                connection.handle(),
                schema.as_ptr(),
                key.as_ptr().cast(),
                key.len() as i32,
            )
        };
        ensure!(
            result == rusqlite::ffi::SQLITE_OK,
            "Could not initialize cache encryption. The original database was kept."
        );
        Ok(())
    }

    pub fn open(&self, path: &Path, flags: OpenFlags) -> anyhow::Result<Connection> {
        let connection = Connection::open_with_flags(path, flags)?;
        self.initialize(&connection)?;
        Ok(connection)
    }
}

#[cfg(test)]
mod temp_policy_tests;
#[cfg(test)]
mod tests;

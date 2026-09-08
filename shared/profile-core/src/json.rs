use crate::{Error, Result};
use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::fmt;

pub fn decode(bytes: &[u8]) -> Result<Value> {
    serde_json::from_slice::<Unique>(bytes)
        .map(|v| v.0)
        .map_err(|_| Error::Invalid)
}

pub fn encode(value: &impl serde::Serialize) -> Result<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        full: bool,
    }
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > crate::MAX_RECORD_BYTES - self.bytes.len() {
                self.full = true;
                return Err(std::io::Error::other("profile record size"));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded {
        bytes: Vec::new(),
        full: false,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| {
        if output.full {
            Error::TooLarge
        } else {
            Error::Invalid
        }
    })?;
    Ok(output.bytes)
}
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("profile JSON")
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|v| Unique(Value::Number(v)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(v)) = seq.next_element()? {
                    values.push(v);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Map::new();
                while let Some((key, Unique(value))) = map.next_entry::<String, Unique>()? {
                    if values.insert(key, value).is_some() {
                        return Err(serde::de::Error::custom("duplicate field"));
                    }
                }
                Ok(Unique(Value::Object(values)))
            }
        }
        d.deserialize_any(UniqueVisitor)
    }
}

/// A guard against accidentally feeding whole local settings/account state into
/// extension fields. This is not a general secret detector: exporters still
/// need explicit field mappings. Protected credentials need a separate codec.
pub fn portable(value: &Value) -> Result<()> {
    portable_depth(value, 0)
}
pub fn portable_map(values: &Map<String, Value>) -> Result<()> {
    portable_fields(values, 0)
}
fn portable_depth(value: &Value, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(Error::Invalid);
    }
    match value {
        Value::Array(values) => {
            for value in values {
                portable_depth(value, depth + 1)?;
            }
        }
        Value::Object(values) => portable_fields(values, depth)?,
        _ => {}
    }
    Ok(())
}
fn portable_fields(values: &Map<String, Value>, depth: usize) -> Result<()> {
    for (key, value) in values {
        if [
            "password",
            "smtp_password",
            "access_token",
            "refresh_token",
            "client_secret",
            "google_client_secret",
            "credential_slot",
            "google_grant",
            "google_lifecycle",
            "last_backup",
            "backup_ready",
            "device_path",
            "delivery_journal",
            "mail_uid",
            "window_position",
        ]
        .contains(&key.as_str())
        {
            return Err(Error::LocalData);
        }
        portable_depth(value, depth + 1)?;
    }
    Ok(())
}

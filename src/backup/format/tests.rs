use super::*;

fn snapshot() -> Snapshot {
    Snapshot {
        version: 1,
        created_at: 1,
        messages: vec![],
        accounts: vec![],
        calendars: vec![],
        preferences: crate::model::Preferences::default(),
        credentials: vec![],
    }
}
fn secret() -> SecretString {
    "a format fixture passphrase".into()
}

#[test]
fn backup_format_all_options_round_trip_and_legacy_stays_readable() {
    let snapshot = snapshot();
    for compression in [Compression::Zstd, Compression::None] {
        for protection in [Protection::Passphrase, Protection::None] {
            let options = Options {
                compression,
                protection,
            };
            let password = secret();
            let password = options.encrypted().then_some(&password);
            let encoded = encode(&snapshot, options, password).unwrap();
            assert!(encoded.starts_with(MAGIC));
            assert_eq!(verify(&encoded, password).unwrap(), options);
            assert_eq!(decode(&encoded, password).unwrap().created_at, 1);
            if options.encrypted() {
                assert!(decode(&encoded, None).is_err());
                assert!(decode(&encoded, Some(&"different passphrase".into())).is_err());
                assert!(!encoded.windows(11).any(|bytes| bytes == b"created_at\""));
            }
        }
    }
    let old = crate::backup::encrypt(&snapshot, &secret()).unwrap();
    assert_eq!(verify(&old, Some(&secret())).unwrap(), Options::default());
    assert_eq!(decode(&old, Some(&secret())).unwrap().created_at, 1);
}

#[test]
fn backup_format_stream_rejects_reordering_truncation_header_changes_and_trailing_data() {
    for protection in [Protection::Passphrase, Protection::None] {
        let options = Options {
            compression: Compression::None,
            protection,
        };
        let passphrase = secret();
        let password = options.encrypted().then_some(&passphrase);
        let mut encoded = Encoder::new(Vec::new(), options, password).unwrap();
        encoded.write_all(&vec![b'a'; CHUNK]).unwrap();
        encoded.write_all(&vec![b'b'; CHUNK]).unwrap();
        encoded.write_all(b"final segment").unwrap();
        let encoded = encoded.finish().unwrap();
        assert_eq!(verify(&encoded, password).unwrap(), options);
        let frame = 4 + CHUNK + if options.encrypted() { 16 } else { 0 };
        let mut reordered = encoded.clone();
        reordered[HEADER..HEADER + frame]
            .copy_from_slice(&encoded[HEADER + frame..HEADER + frame * 2]);
        reordered[HEADER + frame..HEADER + frame * 2]
            .copy_from_slice(&encoded[HEADER..HEADER + frame]);
        assert!(verify(&reordered, password).is_err());
        for end in [0, 8, HEADER - 1, HEADER, HEADER + frame, encoded.len() - 1] {
            assert!(verify(&encoded[..end], password).is_err());
        }
        for index in [8, 9, 16, HEADER + 3, HEADER + 10, encoded.len() - 1] {
            let mut corrupted = encoded.clone();
            corrupted[index] ^= 1;
            assert!(
                verify(&corrupted, password).is_err(),
                "accepted corruption at {index}"
            );
        }
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(verify(&trailing, password).is_err());
    }
}

#[test]
fn backup_format_bounds_each_sink_write_and_rejects_plaintext_credentials() {
    struct BoundedSink(usize);
    impl Write for BoundedSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(bytes.len() <= CHUNK + 16);
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut encoder = Encoder::new(BoundedSink(0), Options::default(), Some(&secret())).unwrap();
    io::copy(&mut io::repeat(42).take(CHUNK as u64 * 10), &mut encoder).unwrap();
    assert!(encoder.finish().unwrap().0 > CHUNK * 10);
    let mut snapshot = snapshot();
    snapshot
        .credentials
        .push(("fixture".into(), "secret".into()));
    assert!(
        encode(
            &snapshot,
            Options {
                protection: Protection::None,
                ..Default::default()
            },
            None
        )
        .unwrap_err()
        .to_string()
        .contains("passwords require")
    );
}

#[test]
fn backup_format_restore_requires_final_authentication_and_rejects_exposed_credentials() {
    for compression in [Compression::Zstd, Compression::None] {
        for protection in [Protection::Passphrase, Protection::None] {
            let options = Options {
                compression,
                protection,
            };
            let passphrase = secret();
            let password = options.encrypted().then_some(&passphrase);
            let encoded = encode(&snapshot(), options, password).unwrap();
            for end in [HEADER, encoded.len() - 1, encoded.len() - 16] {
                assert!(decode(&encoded[..end], password).is_err());
            }
            let mut trailing = encoded.clone();
            trailing.push(0);
            assert!(decode(&trailing, password).is_err());
            let mut invalid_length = encoded;
            invalid_length[HEADER..HEADER + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(decode(&invalid_length, password).is_err());
        }
    }
    let mut exposed = snapshot();
    exposed
        .credentials
        .push(("fixture".into(), "secret".into()));
    // Even a checksum-valid file assembled externally cannot import exposed keys.
    let mut encoder = Encoder::new(
        Vec::new(),
        Options {
            compression: Compression::None,
            protection: Protection::None,
        },
        None,
    )
    .unwrap();
    serde_json::to_writer(&mut encoder, &exposed).unwrap();
    let bytes = encoder.finish().unwrap();
    assert!(
        decode(&bytes, None)
            .err()
            .unwrap()
            .to_string()
            .contains("passwords require")
    );
}

#[test]
fn backup_format_encrypted_frames_cannot_be_spliced_between_copies() {
    let options = Options {
        compression: Compression::None,
        ..Default::default()
    };
    let password = secret();
    let prepare = || {
        let mut encoder = Encoder::new(Vec::new(), options, Some(&password)).unwrap();
        encoder.write_all(&vec![42; CHUNK * 2]).unwrap();
        encoder.finish().unwrap()
    };
    let first = prepare();
    let mut second = prepare();
    let end = HEADER + 4 + CHUNK + 16;
    second[HEADER..end].copy_from_slice(&first[HEADER..end]);
    assert!(verify(&second, Some(&password)).is_err());
}

pub(crate) fn wire_fixture(protection: Protection) -> Vec<u8> {
    encode(
        &snapshot(),
        Options {
            protection,
            ..Default::default()
        },
        (protection == Protection::Passphrase).then_some(&secret()),
    )
    .unwrap()
}

#[test]
fn backup_format_matches_independent_libargon2_and_aesgcm_vector() {
    let hex = include_str!("../../../tests/fixtures/backup-format-v2.hex").trim();
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let snapshot = decode(&bytes, Some(&secret())).unwrap();
    assert_eq!(snapshot.created_at, 1);
    assert_eq!(
        verify(&bytes, Some(&secret())).unwrap(),
        Options {
            compression: Compression::None,
            protection: Protection::Passphrase,
        }
    );
}

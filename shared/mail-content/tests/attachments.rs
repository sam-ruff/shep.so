use base64::Engine;
use serde_json::Value;
use shep_mail_content::attachments::{catalog, filename, read};
#[test]
fn shared_attachment_metadata_and_exact_bytes_match() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../attachment-fixtures.json")).unwrap();
    for case in fixtures.as_array().unwrap() {
        let raw = case["raw"].as_str().unwrap().as_bytes();
        let files = catalog(raw).unwrap();
        assert_eq!(files.len(), case["files"].as_array().unwrap().len());
        for (file, expected) in files.iter().zip(case["files"].as_array().unwrap()) {
            assert_eq!(file.id, expected["id"]);
            assert_eq!(file.name, expected["name"]);
            assert_eq!(file.media_type, expected["media_type"]);
            assert_eq!(file.size, expected["size"].as_u64().unwrap() as usize);
            let (info, bytes) = read(raw, &file.id).unwrap();
            assert_eq!(info, *file);
            assert_eq!(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                expected["bytes"]
            );
        }
        assert!(read(raw, "unknown").is_err());
    }
    let raw = fixtures[0]["raw"].as_str().unwrap();
    let files = catalog(raw.as_bytes()).unwrap();
    let changed = raw.replace("AP8BDQo=", "AAECAwQ=");
    assert_ne!(catalog(changed.as_bytes()).unwrap()[0].id, files[0].id);
    assert!(read(changed.as_bytes(), &files[0].id).is_err());
}
#[test]
fn suggested_names_never_contain_sender_paths_or_control_characters() {
    for (input, expected) in [
        ("../../file.txt", "file.txt"),
        ("C:\\temp\\evil.txt", "evil.txt"),
        ("\u{202e}r\0ésumé.txt", "résumé.txt"),
        ("..", "attachment.bin"),
        ("/", "attachment.bin"),
    ] {
        assert_eq!(filename(input), expected);
    }
}

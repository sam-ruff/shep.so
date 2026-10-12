use anyhow::Result;
use serde_json::Value;
use shep_mail_core::model::Mail;

pub(crate) fn display_metadata(message: &Mail) -> Result<Value> {
    let mut value = serde_json::to_value(message)?;
    value["sender_address"] = sender_address(&message.sender).into();
    Ok(value)
}

fn sender_address(header: &str) -> String {
    if header.len() > 16 * 1024 || header.contains(['\r', '\n', '\0']) {
        return String::new();
    }
    shep_mail_core::compose::recipients(header.trim(), "From")
        .ok()
        .and_then(|mut addresses| {
            if addresses.len() != 1 {
                return None;
            }
            addresses.pop()
        })
        .map_or_else(String::new, |mailbox| mailbox.email.to_string())
}

#[cfg(test)]
mod tests {
    use super::sender_address;
    #[test]
    fn quoted_angle_brackets_and_escapes_cannot_become_an_address() {
        assert_eq!(
            sender_address(r#" "Help <desk>" <Sender@Example.TEST> "#),
            "Sender@Example.TEST"
        );
        assert_eq!(
            sender_address(r#""Robin \"RJ\" Field" <sender@example.test>"#),
            "sender@example.test"
        );
    }
    #[test]
    fn ambiguous_missing_invalid_and_oversized_headers_never_guess_an_address() {
        for header in [
            "",
            "   ",
            "Display name only",
            "first@example.test, second@example.test",
            "sender@example.test\r\nBcc: hidden@example.test",
            "sender@example.test\0",
            &"a".repeat(16 * 1024 + 1),
        ] {
            assert_eq!(sender_address(header), "", "{header}");
        }
    }
}

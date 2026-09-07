//! Fixture/tooling adapter: prepare stdin MIME without embedding fixture mail.
use std::io::{Read, Write};
fn main() -> anyhow::Result<()> {
    let options = serde_json::from_str(
        &std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Supply document options as JSON"))?,
    )?;
    let mut raw = Vec::new();
    std::io::stdin()
        .take(shep_mail_content::MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut raw)?;
    let result = shep_mail_content::document::prepare(&raw, &options)?;
    std::io::stdout().write_all(&serde_json::to_vec(&result)?)?;
    Ok(())
}

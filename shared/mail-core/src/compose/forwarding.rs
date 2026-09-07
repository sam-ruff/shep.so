//! Attach platform-independent forward content to a new independent draft.
use super::FilePart;
use crate::model::{Draft, DraftAttachment};
pub use shep_mail_content::forwarding::ForwardQuote;

pub fn prepare_forward(
    id: String,
    account: String,
    raw: &[u8],
) -> anyhow::Result<(Draft, Vec<FilePart>)> {
    let content = shep_mail_content::forwarding::prepare(raw)?;
    let files: Vec<_> = content
        .files
        .into_iter()
        .map(|file| FilePart {
            attachment: DraftAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name: file.name,
                media_type: file.media_type,
                size: file.bytes.len(),
                content_id: file.content_id,
            },
            bytes: file.bytes,
        })
        .collect();
    let draft = Draft {
        id,
        account_id: account,
        subject: content.subject,
        body: content.body,
        forward: Some(content.forward),
        revision: 1,
        attachments: files.iter().map(|f| f.attachment.clone()).collect(),
        ..Default::default()
    };
    Ok((draft, files))
}

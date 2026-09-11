use crate::model::{StoredMail, parse_mail};

const CASES: &[(&str, &str)] = &[
    ("Roadmap agenda", "project review"),
    (
        "Project review",
        "Notes from the product team and the launch agenda.",
    ),
    ("Release planning notes", "A draft agenda for our launch."),
    (
        "Planning the release",
        "The release scope and planning dates are ready.",
    ),
    (
        "Conference booking",
        "Conference accommodation for the September visit.",
    ),
    (
        "Quiet reading corner",
        "The reading room is available on Friday.",
    ),
    (
        "Delivery address",
        "Please use the new office address for the parcel.",
    ),
    ("Adviser meeting", "The adviser can meet on Monday."),
    ("Café reservation", "We have a table by the window."),
    ("Invoice 2026", "Invoice number 2026 covers the room hire."),
    ("Invoice 2027", "Invoice number 2027 covers the room hire."),
    (
        "Archive permissions",
        "The project archive is ready for review.",
    ),
];

pub(super) fn populate(mails: &mut [StoredMail]) -> anyhow::Result<()> {
    let start = mails
        .len()
        .checked_sub(CASES.len())
        .ok_or_else(|| anyhow::anyhow!("Search comparison needs twelve generic fixture rows"))?;
    for (mail, (subject, body)) in mails[start..].iter_mut().zip(CASES) {
        let original = &mail.summary;
        let timestamp = original.timestamp;
        let raw = format!(
            "From: Morgan <morgan@studio.example>\r\nTo: alex@studio.example\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
        );
        let mut replacement = parse_mail(
            &original.account_id,
            &original.remote_id,
            &original.folder,
            raw.into_bytes(),
            original.unread,
            original.starred,
        )?;
        replacement.summary.timestamp = timestamp;
        *mail = replacement;
    }
    Ok(())
}

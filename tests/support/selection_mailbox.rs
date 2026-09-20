use crate::{model::parse_mail, store::Store};

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let date = (chrono::Utc::now() - chrono::Duration::days(365)).to_rfc2822();
    for start in (120..100_000).step_by(500) {
        let mut messages = Vec::with_capacity(500);
        for index in start..(start + 500).min(100_000) {
            messages.push(parse_mail(
                "preview-work",
                &format!("1.{index}"),
                "INBOX",
                format!("From: Archive <archive@example.test>\r\nTo: alex@studio.example\r\nSubject: Saved project {index}\r\nDate: {date}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA fictional saved conversation.\r\n").into_bytes(),
                false,
                false,
            )?);
        }
        store.run(move |connection| {
            let transaction = connection.transaction()?;
            {
                let mut insert = transaction.prepare_cached("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw) VALUES(?1,?2,?3,?4,?5,?6,?7,0,0,?8,?9)")?;
                for message in messages {
                    let mail = &message.summary;
                    insert.execute(rusqlite::params![mail.id,mail.account_id,mail.folder,mail.sender,mail.subject,message.text,mail.timestamp,serde_json::to_string(mail)?,message.raw])?;
                }
            }
            transaction.commit()?;
            Ok(())
        }).await?;
    }
    Ok(())
}

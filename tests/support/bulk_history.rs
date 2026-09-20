//! Prior fictional groups and a paused group for native History controls.
use crate::{
    bulk::{Action, Receipt},
    mail_actions::Flags,
    model::MailQuery,
    store::{MailSelectionId, SelectionChange, Store},
};
use anyhow::Context;

pub async fn seed_flag_repair(store: &Store) -> anyhow::Result<()> {
    let original = store
        .query(MailQuery::default())
        .await?
        .rows
        .into_iter()
        .next()
        .context("Fixture message missing")?;
    let flags = Flags {
        unread: None,
        starred: Some(!original.starred),
    };
    store
        .start_individual_mail_action(
            "flag-repair-fixture".into(),
            original.clone(),
            Action::Flags(flags),
        )
        .await?;
    let item = store
        .claim_bulk_item("flag-repair-fixture".into())
        .await?
        .context("Fixture claim missing")?;
    store
        .acknowledge_bulk_flags(
            item,
            Receipt::Flags {
                before: Flags {
                    unread: None,
                    starred: Some(original.starred),
                },
                after: flags,
            },
        )
        .await?;
    store
        .run(|c| {
            c.execute(
                "UPDATE bulk_jobs SET paused=1 WHERE id='flag-repair-fixture'",
                [],
            )?;
            Ok(())
        })
        .await
}

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let query = MailQuery {
        folder: "INBOX".into(),
        ..Default::default()
    };
    let rows = store.query(query.clone()).await?.rows;
    let selected = store
        .capture_selection(MailSelectionId::default(), 0, query.clone(), false, vec![])
        .await?;
    let many = store
        .capture_selection(MailSelectionId::default(), 0, query, true, vec![])
        .await?;
    let review = store.freeze_selection(many.id, many.revision).await?;
    store
        .start_bulk(
            "paged-fixture".into(),
            review.id,
            Action::Move {
                account: None,
                folder: "INBOX".into(),
            },
        )
        .await?;
    while let Some(item) = store.claim_bulk_item("paged-fixture".into()).await? {
        store.finish_bulk_item(item, Ok(Receipt::Unchanged)).await?;
    }
    store
        .run(|c| {
            c.execute(
                "UPDATE bulk_jobs SET created=24 WHERE id='paged-fixture'",
                [],
            )?;
            Ok(())
        })
        .await?;
    store.release_selection(many.id).await?;
    let selected = store
        .change_selection(
            selected.id,
            selected.revision,
            SelectionChange::Set {
                id: rows[0].id.clone(),
                selected: true,
                clear_others: false,
            },
            vec![],
        )
        .await?;
    let action = Action::Flags(Flags {
        unread: None,
        starred: Some(true),
    });
    for index in 0..24 {
        let review = store
            .freeze_selection(selected.id, selected.revision)
            .await?;
        let id = format!("history-{index:02}");
        store
            .start_bulk(id.clone(), review.id, action.clone())
            .await?;
        let item = store.claim_bulk_item(id.clone()).await?.unwrap();
        store.finish_bulk_item(item, Ok(Receipt::Unchanged)).await?;
        store
            .run(move |c| {
                c.execute(
                    "UPDATE bulk_jobs SET created=? WHERE id=?",
                    rusqlite::params![index, id],
                )?;
                Ok(())
            })
            .await?;
    }
    let selected = store
        .change_selection(
            selected.id,
            selected.revision,
            SelectionChange::Set {
                id: rows[1].id.clone(),
                selected: true,
                clear_others: false,
            },
            vec![],
        )
        .await?;
    let review = store
        .freeze_selection(selected.id, selected.revision)
        .await?;
    store
        .start_bulk("paused-fixture".into(), review.id, action)
        .await?;
    store
        .run(|c| {
            c.execute(
                "UPDATE bulk_jobs SET paused=1 WHERE id='paused-fixture'",
                [],
            )?;
            Ok(())
        })
        .await?;
    store.release_selection(review.id).await?;
    store.release_selection(selected.id).await?;
    Ok(())
}

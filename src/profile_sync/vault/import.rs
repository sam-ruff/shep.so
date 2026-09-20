//! Staged activation of passwords received from another device. The pair is
//! staged in new keychain slots, tested against this account's own servers and
//! only then activated with the exact synced revisions.
use super::*;
use anyhow::Context as _;

#[derive(Debug)]
pub struct Import {
    pub setup: String,
    pub local: String,
    pub shared: Uuid,
    /// The native account as reconciled; activation refuses if it changed.
    pub account: Account,
    pub pair: Vec<(Field, SecretString)>,
    pub revisions: Vec<(Field, u64)>,
}

/// Write the pair to staging slots and read it back from the keychain.
pub async fn stage(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    ctx.store
        .check_credential_import(import.local.clone(), import.shared, import.account.clone())
        .await?;
    let attempt = ctx
        .store
        .admit_account_setup(
            import.setup.clone(),
            import.account.clone(),
            Some(import.account.clone()),
        )
        .await?;
    if matches!(
        attempt.stage,
        crate::store::account_setup::Stage::Activated
            | crate::store::account_setup::Stage::Checked
            | crate::store::account_setup::Stage::Staged
    ) {
        return Ok(());
    }
    ctx.store
        .mark_credentials_staged(import.local.clone(), true)
        .await?;
    let incoming = import
        .pair
        .iter()
        .find(|(field, _)| *field == Field::Incoming)
        .context("The imported incoming credential is missing")?
        .1
        .clone();
    let smtp = import
        .pair
        .iter()
        .find(|(field, _)| *field == Field::Smtp)
        .map(|(_, secret)| secret.clone());
    ctx.credentials
        .stage_account_setup(ctx.store, import.setup.clone(), incoming, smtp)
        .await?;
    Ok(())
}

/// Test the staged pair: incoming always, SMTP when it uses its own password.
pub async fn test(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    let attempt = ctx
        .store
        .validate_account_setup(import.setup.clone())
        .await?;
    if matches!(
        attempt.stage,
        crate::store::account_setup::Stage::Activated | crate::store::account_setup::Stage::Checked
    ) {
        return Ok(());
    }
    for field in fields(&import.account) {
        let secret = ctx
            .credentials
            .read(match field {
                Field::Incoming => &attempt.slots.incoming,
                Field::Smtp => attempt
                    .slots
                    .smtp
                    .as_ref()
                    .context("Separate SMTP staging is missing")?,
            })
            .await?;
        let target = match field {
            Field::Incoming => ConnectionTarget::Incoming,
            Field::Smtp => ConnectionTarget::Smtp,
        };
        ctx.tester.test(&import.account, target, &secret).await?;
    }
    ctx.store
        .advance_account_setup(
            import.setup.clone(),
            crate::store::account_setup::Stage::Staged,
            crate::store::account_setup::Stage::Checked,
        )
        .await?;
    Ok(())
}

/// Callers hold the connection lifecycle and account locks during activation.
pub async fn activate(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    ctx.store
        .check_credential_import(import.local.clone(), import.shared, import.account.clone())
        .await?;
    ctx.store
        .activate_credentials(
            import.local.clone(),
            import.shared,
            import.account.clone(),
            import.revisions.clone(),
            import.setup.clone(),
        )
        .await
}

/// Keep the active pair and do not retry the same revision automatically.
pub async fn record_failure(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    ctx.store
        .fail_account_setup(
            import.setup.clone(),
            "The imported connection check failed. The previous credentials were kept.".into(),
        )
        .await?;
    ctx.store
        .fail_credential_import(import.shared, import.revisions.clone())
        .await
}

pub async fn unstage(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    let attempt = ctx.store.account_setup(import.setup.clone()).await?;
    if attempt.stage != crate::store::account_setup::Stage::Activated {
        ctx.store
            .fail_account_setup(
                import.setup.clone(),
                "Credential activation did not finish. Review account setup before retrying."
                    .into(),
            )
            .await?;
    }
    for field in [Field::Incoming, Field::Smtp] {
        ctx.credentials
            .delete(&staged_key(&import.local, field))
            .await?;
    }
    ctx.store
        .mark_credentials_staged(import.local.clone(), false)
        .await
}

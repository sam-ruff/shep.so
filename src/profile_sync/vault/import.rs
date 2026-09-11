//! Staged activation of passwords received from another device. The pair is
//! staged in new keychain slots, tested against this account's own servers and
//! only then copied over the active slots.
use super::*;
use secrecy::ExposeSecret;

#[derive(Debug)]
pub struct Import {
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
        .mark_credentials_staged(import.local.clone(), true)
        .await?;
    for (field, secret) in &import.pair {
        let key = staged_key(&import.local, *field);
        ctx.credentials.write(&key, secret.clone()).await?;
        let saved = ctx.credentials.read(&key).await?;
        anyhow::ensure!(
            saved.expose_secret() == secret.expose_secret(),
            "The keychain did not keep the synced password. The previous password was kept."
        );
    }
    Ok(())
}

/// Test the staged pair: incoming always, SMTP when it uses its own password.
pub async fn test(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    for field in fields(&import.account) {
        let secret = ctx
            .credentials
            .read(&staged_key(&import.local, field))
            .await?;
        let target = match field {
            Field::Incoming => ConnectionTarget::Incoming,
            Field::Smtp => ConnectionTarget::Smtp,
        };
        ctx.tester.test(&import.account, target, &secret).await?;
    }
    Ok(())
}

/// Callers hold the connection lifecycle and account locks, as SaveAccount
/// does, so the checked account cannot change before the keychain write.
pub async fn activate(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    ctx.store
        .check_credential_import(import.local.clone(), import.shared, import.account.clone())
        .await?;
    // SMTP first, then incoming, matching SaveAccount's write order.
    for field in fields(&import.account).into_iter().rev() {
        let staged = ctx
            .credentials
            .read(&staged_key(&import.local, field))
            .await?;
        ctx.credentials
            .write(&keychain_key(&import.local, field), staged)
            .await?;
    }
    ctx.store
        .activate_credentials(
            import.local.clone(),
            import.shared,
            import.account.clone(),
            import.revisions.clone(),
        )
        .await
}

/// Keep the active pair and do not retry the same revision automatically.
pub async fn record_failure(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    ctx.store
        .fail_credential_import(import.shared, import.revisions.clone())
        .await
}

pub async fn unstage(ctx: &Context<'_>, import: &Import) -> anyhow::Result<()> {
    for field in [Field::Incoming, Field::Smtp] {
        ctx.credentials
            .delete(&staged_key(&import.local, field))
            .await?;
    }
    ctx.store
        .mark_credentials_staged(import.local.clone(), false)
        .await
}

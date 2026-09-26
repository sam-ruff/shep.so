fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mailto = shep::mailto::from_args(std::env::args_os().skip(1));
    let launch = shep::activation::start(mailto.clone())?;
    let signal = match &launch {
        shep::activation::Launch::Primary(owner) => Some(owner.signal()),
        shep::activation::Launch::Activated => return Ok(()),
        shep::activation::Launch::Independent => None,
        shep::activation::Launch::Stale => {
            tracing::warn!("{}", shep::activation::STALE_OWNER_NOTICE);
            return Ok(shep::ui::update_notice::run()?);
        }
    };
    shep::ui::run(signal, mailto)?;
    drop(launch);
    Ok(())
}

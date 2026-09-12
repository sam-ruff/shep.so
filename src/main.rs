fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let launch = shep::activation::start()?;
    let signal = match &launch {
        shep::activation::Launch::Primary(owner) => Some(owner.signal()),
        shep::activation::Launch::Activated => return Ok(()),
        shep::activation::Launch::Independent => None,
    };
    shep::ui::run(signal)?;
    drop(launch);
    Ok(())
}

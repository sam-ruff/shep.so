use shep_beta_server::{AppState, app, config::Config, google::Google, profiles::GoogleProfiles};
use std::sync::Arc;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::from_env()?);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let verifier = Arc::new(Google::new(config.clone())?);
    let provider = Arc::new(GoogleProfiles::new(config.clone(), verifier.clone())?);
    let router = app(AppState::new(config, verifier, provider));
    // Do not log request URLs, callback codes, cookies, headers or bodies.
    println!("Shep beta gateway listening on configured loopback address");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

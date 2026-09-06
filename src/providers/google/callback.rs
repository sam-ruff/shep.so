//! Bounded loopback callback parsing; request contents never reach logs/errors.
use anyhow::Context;
use secrecy::SecretString;
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

enum Callback {
    Code(SecretString),
    Denied,
}

pub(super) async fn receive(
    listener: TcpListener,
    expected_state: &str,
) -> anyhow::Result<SecretString> {
    let authority = listener.local_addr()?.to_string();
    let mut requests = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            result = requests.join_next(), if !requests.is_empty() => {
                match result {
                    Some(Ok(Ok(Ok(Some(Callback::Code(code)))))) => return Ok(code),
                    Some(Ok(Ok(Ok(Some(Callback::Denied))))) => anyhow::bail!("Google sign-in was cancelled or denied. Try connecting again."),
                    _ => {} // Ignore malformed, idle or disconnected local clients.
                }
            }
            accepted = listener.accept(), if requests.len() < 8 => {
                let (stream, _) = accepted?;
                let state = expected_state.to_owned();
                let authority = authority.clone();
                requests.spawn(async move {
                    tokio::time::timeout(Duration::from_secs(5), request(stream, &authority, &state)).await
                });
            }
        }
    }
}

async fn request(
    mut stream: TcpStream,
    authority: &str,
    state: &str,
) -> anyhow::Result<Option<Callback>> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > 8192 {
            respond(&mut stream, false, "Sign-in response is too large.").await;
            return Ok(None);
        }
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let callback = parse(&bytes, authority, state).ok();
    match callback {
        Some(Callback::Code(_)) => {
            respond(
                &mut stream,
                true,
                "Sign-in received. You can return to Shep.",
            )
            .await
        }
        Some(Callback::Denied) => {
            respond(
                &mut stream,
                true,
                "Sign-in was cancelled. You can return to Shep.",
            )
            .await
        }
        None => respond(&mut stream, false, "Invalid sign-in response.").await,
    }
    Ok(callback)
}

fn parse(bytes: &[u8], authority: &str, expected_state: &str) -> anyhow::Result<Callback> {
    let request = std::str::from_utf8(bytes).context("Invalid callback")?;
    let mut lines = request.split("\r\n");
    let first: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    anyhow::ensure!(
        first.len() == 3 && first[0] == "GET" && matches!(first[2], "HTTP/1.1" | "HTTP/1.0"),
        "Invalid callback"
    );
    anyhow::ensure!(
        first[1].starts_with("/callback?") && !first[1].contains('#'),
        "Invalid callback"
    );
    let mut host = None;
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').context("Invalid callback")?;
        if name.eq_ignore_ascii_case("host") {
            anyhow::ensure!(host.replace(value.trim()).is_none(), "Invalid callback");
        }
    }
    anyhow::ensure!(host == Some(authority), "Invalid callback");
    let url = url::Url::parse(&format!("http://{authority}{}", first[1]))?;
    let mut args = HashMap::new();
    for (key, value) in url.query_pairs() {
        anyhow::ensure!(
            args.insert(key.into_owned(), value.into_owned()).is_none(),
            "Invalid callback"
        );
    }
    anyhow::ensure!(
        args.get("state").map(String::as_str) == Some(expected_state),
        "Invalid callback"
    );
    match (args.remove("code"), args.remove("error")) {
        (Some(code), None)
            if !code.is_empty() && code.bytes().all(|byte| byte.is_ascii_graphic()) =>
        {
            Ok(Callback::Code(SecretString::from(code)))
        }
        (None, Some(error)) if !error.is_empty() => Ok(Callback::Denied),
        _ => anyhow::bail!("Invalid callback"),
    }
}

async fn respond(stream: &mut TcpStream, success: bool, body: &str) {
    let status = if success { "200 OK" } else { "400 Bad Request" };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    // A browser closing after sending a valid callback must not lose the code.
    let _ = stream.write_all(response.as_bytes()).await;
}

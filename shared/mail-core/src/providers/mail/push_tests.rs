use super::push::{Push, REISSUE, WatchEnd, watch_inbox, watch_session};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Notify,
    time::Instant,
};

/// Commands the fake server saw, with the virtual time they arrived.
type Log = Arc<Mutex<Vec<(String, Duration)>>>;

/// A fake IMAP server transcript. Each step names the expected command (DONE
/// has no tag) and the reply, where `$TAG` is the latest tagged command's tag.
struct Script {
    steps: Vec<(&'static str, &'static str)>,
    /// Fires the stop signal once the server has seen this many commands.
    stop_after: Option<usize>,
    /// Drops the connection after the script instead of expecting a close.
    hang_up: bool,
}

async fn serve(
    server: tokio::io::DuplexStream,
    script: Script,
    log: Log,
    stop: Arc<Notify>,
    start: Instant,
) {
    let mut server = BufReader::new(server);
    server
        .get_mut()
        .write_all(b"* OK fixture\r\n")
        .await
        .expect("greeting");
    let mut line = String::new();
    server.read_line(&mut line).await.expect("login");
    let mut tag = line.split_whitespace().next().expect("tag").to_owned();
    server
        .get_mut()
        .write_all(format!("{tag} OK login\r\n").as_bytes())
        .await
        .expect("login response");
    for (expected, response) in script.steps {
        line.clear();
        server.read_line(&mut line).await.expect("command");
        let command = match line.trim_end().split_once(' ') {
            Some((seen, command)) if expected != "DONE" => {
                tag = seen.to_owned();
                command.to_owned()
            }
            _ => line.trim_end().to_owned(),
        };
        let seen = {
            let mut log = log.lock().expect("log");
            log.push((command.clone(), start.elapsed()));
            log.len()
        };
        assert_eq!(command, expected);
        if Some(seen) == script.stop_after {
            stop.notify_one();
        }
        server
            .get_mut()
            .write_all(response.replace("$TAG", &tag).as_bytes())
            .await
            .expect("response");
    }
    if script.hang_up {
        return;
    }
    line.clear();
    let bytes = tokio::time::timeout(Duration::from_secs(1), server.read_line(&mut line))
        .await
        .expect("client closes after the script")
        .expect("read final input");
    assert_eq!(bytes, 0, "Unexpected command after the script: {line}");
}

struct Run {
    result: anyhow::Result<WatchEnd>,
    pushes: Vec<(Push, Duration)>,
    commands: Vec<(String, Duration)>,
}

impl Run {
    fn names(&self) -> Vec<&str> {
        self.commands
            .iter()
            .map(|(command, _)| command.as_str())
            .collect()
    }
}

async fn run(steps: Vec<(&'static str, &'static str)>, stop_after: Option<usize>) -> Run {
    run_script(Script {
        steps,
        stop_after,
        hang_up: false,
    })
    .await
}

async fn run_script(script: Script) -> Run {
    let start = Instant::now();
    let (client, server) = tokio::io::duplex(8192);
    let log: Log = Default::default();
    let stop = Arc::new(Notify::new());
    let peer = serve(server, script, log.clone(), stop.clone(), start);
    let pushes = Arc::new(Mutex::new(Vec::new()));
    let seen = pushes.clone();
    let client = async {
        let mut client = async_imap::Client::new(client);
        client.read_response().await.expect("greeting");
        let session = client.login("fixture", "fixture").await.expect("login");
        watch_session(
            session,
            |push| seen.lock().expect("pushes").push((push, start.elapsed())),
            stop.notified(),
        )
        .await
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(3600), async {
        tokio::join!(client, peer)
    })
    .await
    .expect("the scripted session completes");
    let pushes = pushes.lock().expect("pushes").clone();
    let commands = log.lock().expect("log").clone();
    Run {
        result,
        pushes,
        commands,
    }
}

const CAPABLE: (&str, &str) = (
    "CAPABILITY",
    "* CAPABILITY IMAP4rev1 IDLE CONDSTORE QRESYNC\r\n$TAG OK done\r\n",
);
const SELECT: (&str, &str) = (
    "SELECT \"INBOX\"",
    "* 4 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n$TAG OK [READ-WRITE] selected\r\n",
);
const IDLING: (&str, &str) = ("IDLE", "+ idling\r\n");
const DONE: (&str, &str) = ("DONE", "$TAG OK idle finished\r\n");
const LOGOUT: (&str, &str) = ("LOGOUT", "* BYE bye\r\n$TAG OK logged out\r\n");

#[tokio::test(start_paused = true)]
async fn idle_follows_select_and_an_exists_reports_a_change() {
    let run = run(
        vec![
            CAPABLE,
            SELECT,
            ("IDLE", "+ idling\r\n* 5 EXISTS\r\n"),
            DONE,
            IDLING,
            DONE,
            LOGOUT,
        ],
        Some(5),
    )
    .await;
    assert_eq!(
        run.names(),
        [
            "CAPABILITY",
            SELECT.0,
            "IDLE",
            "DONE",
            "IDLE",
            "DONE",
            "LOGOUT"
        ]
    );
    assert_eq!(
        run.pushes.iter().map(|(push, _)| *push).collect::<Vec<_>>(),
        [Push::Connected, Push::Changed]
    );
    assert!(
        matches!(run.result, Ok(WatchEnd::Stopped)),
        "{:?}",
        run.result
    );
}

#[tokio::test(start_paused = true)]
async fn stop_sends_done_and_logs_out() {
    let run = run(vec![CAPABLE, SELECT, IDLING, DONE, LOGOUT], Some(3)).await;
    assert_eq!(
        run.names(),
        ["CAPABILITY", SELECT.0, "IDLE", "DONE", "LOGOUT"]
    );
    assert!(
        matches!(run.result, Ok(WatchEnd::Stopped)),
        "{:?}",
        run.result
    );
    assert_eq!(run.pushes.len(), 1, "no change was reported");
    assert!(
        run.commands.last().expect("logout").1 < Duration::from_secs(1),
        "a stop must not wait for the re-issue deadline"
    );
}

#[tokio::test(start_paused = true)]
async fn idle_is_reissued_before_the_server_timeout() {
    let run = run(
        vec![CAPABLE, SELECT, IDLING, DONE, IDLING, DONE, LOGOUT],
        Some(5),
    )
    .await;
    assert_eq!(
        run.names(),
        [
            "CAPABILITY",
            SELECT.0,
            "IDLE",
            "DONE",
            "IDLE",
            "DONE",
            "LOGOUT"
        ]
    );
    let first_done = run.commands[3].1;
    assert_eq!(first_done, REISSUE);
    assert!(REISSUE < Duration::from_secs(29 * 60));
    assert_eq!(run.pushes.len(), 1, "a re-issue is not a change");
    assert!(matches!(run.result, Ok(WatchEnd::Stopped)));
}

#[tokio::test(start_paused = true)]
async fn a_server_without_idle_is_reported_as_unsupported() {
    let run = run(
        vec![
            (
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 UIDPLUS MOVE\r\n$TAG OK done\r\n",
            ),
            LOGOUT,
        ],
        None,
    )
    .await;
    assert_eq!(run.names(), ["CAPABILITY", "LOGOUT"]);
    assert!(
        matches!(run.result, Ok(WatchEnd::Unsupported)),
        "{:?}",
        run.result
    );
    assert!(run.pushes.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_dropped_connection_is_an_error_for_the_caller_to_retry() {
    let run = run_script(Script {
        steps: vec![CAPABLE, SELECT, IDLING],
        stop_after: None,
        hang_up: true,
    })
    .await;
    assert_eq!(run.names(), ["CAPABILITY", SELECT.0, "IDLE"]);
    assert!(run.result.is_err(), "{:?}", run.result);
    assert_eq!(
        run.pushes.iter().map(|(push, _)| *push).collect::<Vec<_>>(),
        [Push::Connected]
    );
}

#[tokio::test]
async fn pop3_accounts_are_unsupported_without_connecting() {
    let account: crate::model::Account = serde_json::from_value(serde_json::json!({
        "id": "fixture", "name": "Fixture", "email": "fixture@example.test",
        "protocol": "Pop3", "host": "localhost", "port": 995, "username": "fixture",
        "smtp_host": "localhost", "smtp_port": 465
    }))
    .expect("account");
    let result = watch_inbox(
        &account,
        &secrecy::SecretString::from("secret"),
        |_| {},
        std::future::pending(),
    )
    .await;
    assert!(matches!(result, Ok(WatchEnd::Unsupported)), "{result:?}");
}

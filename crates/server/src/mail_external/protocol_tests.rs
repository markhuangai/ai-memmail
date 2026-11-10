use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::ai::LiveDecisionEngine;
use crate::config::AppConfig;
use crate::logging::{test_capture::capture, StdoutLogger};
use crate::mail::{LiveMailTransport, MailTransport};
use crate::storage::MemoryProcessingStore;

use super::*;

fn config() -> AppConfig {
    AppConfig::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/config.example.yaml"),
    )
    .unwrap()
}

fn mailbox(address: SocketAddr) -> MailboxConfig {
    let mut mailbox = config().mailboxes.remove(0);
    mailbox.imap.host = address.ip().to_string();
    mailbox.imap.port = address.port();
    mailbox.imap.tls = false;
    mailbox.imap.sent_folder = None;
    mailbox.imap.sent_backfill_days = 0;
    mailbox.imap.username = "support".to_string();
    mailbox.imap.password = "synthetic-imap-password".to_string();
    mailbox.accepted_conditions.clear();
    mailbox
}

pub(crate) fn server(
    greeting: &str,
    steps: Vec<(&'static str, String)>,
) -> (SocketAddr, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let greeting = greeting.to_string();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        reader.get_mut().write_all(greeting.as_bytes()).unwrap();
        let mut commands = Vec::new();
        for (expected, response) in steps {
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            let (tag, operation) = command.trim_end().split_once(' ').unwrap();
            assert!(operation.starts_with(expected), "unexpected IMAP operation");
            reader
                .get_mut()
                .write_all(response.replace("$TAG", tag).as_bytes())
                .unwrap();
            commands.push(operation.to_string());
        }
        commands
    });
    (address, handle)
}

fn login() -> (&'static str, String) {
    ("LOGIN ", "$TAG OK authenticated\r\n".to_string())
}
fn select() -> (&'static str, String) {
    (
        "SELECT ",
        "* 1 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n$TAG OK selected\r\n".to_string(),
    )
}
fn logout() -> (&'static str, String) {
    ("LOGOUT", "* BYE done\r\n$TAG OK logged out\r\n".to_string())
}
fn fetch_response(sent: bool) -> String {
    let raw = "From: Person <person@example.com>\r\nTo: support@example.com\r\nSubject: Protocol regression\r\nMessage-ID: <protocol@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nComplete message";
    let date = if sent {
        "INTERNALDATE \"01-Jan-2026 10:00:00 +0000\" "
    } else {
        ""
    };
    format!(
        "* 1 FETCH (UID 7 {date}BODY[] {{{}}}\r\n{raw})\r\n$TAG OK fetched\r\n",
        raw.len()
    )
}

#[test]
fn unseen_fetch_parses_complete_messages_and_drains_the_response() {
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            select(),
            (
                "UID SEARCH UNSEEN",
                "* SEARCH 7 7\r\n$TAG OK searched\r\n".to_string(),
            ),
            ("UID FETCH 7 ", fetch_response(false)),
            logout(),
        ],
    );
    let messages = fetch_unseen_blocking(&mailbox(address), 10).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].plain_text, "Complete message");
    assert_eq!(messages[0].metadata.uid, 7);
    assert_eq!(messages[0].metadata.uid_validity, 9);
    assert_eq!(
        messages[0].metadata.message_id.as_deref(),
        Some("<protocol@example.com>")
    );
    let commands = server.join().unwrap();
    assert!(commands[3].contains("BODY.PEEK[]"));
    assert!(!commands.iter().any(|command| command.contains("STORE")));
}

#[test]
fn sent_fetch_discovers_typed_sent_attribute_and_preserves_date() {
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            (
                "LIST ",
                "* LIST (\\HasNoChildren \\Sent) \"/\" \"Sent Items\"\r\n$TAG OK listed\r\n"
                    .to_string(),
            ),
            (
                "EXAMINE ",
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 55] valid\r\n$TAG OK selected\r\n".to_string(),
            ),
            (
                "UID SEARCH SINCE ",
                "* SEARCH 7\r\n$TAG OK searched\r\n".to_string(),
            ),
            ("UID FETCH 7 ", fetch_response(true)),
            logout(),
        ],
    );
    let batch = fetch_sent_blocking(&mailbox(address), None, 0, 200).unwrap();
    assert_eq!(batch.folder_name, "Sent Items");
    assert_eq!(batch.uid_validity, 55);
    assert!(batch.complete);
    assert_eq!(batch.messages[0].message.plain_text, "Complete message");
    assert!(batch.messages[0].internal_date.is_some());
    assert!(server.join().unwrap()[4].contains("BODY.PEEK[]"));
}

#[test]
fn missing_or_multiple_sent_folders_fail_closed() {
    for folders in [
        "* LIST (\\HasNoChildren) \"/\" \"Inbox\"\r\n",
        "* LIST (\\Sent) \"/\" \"Sent\"\r\n* LIST (\\Sent) \"/\" \"Other\"\r\n",
    ] {
        let (address, server) = server(
            "* OK ready\r\n",
            vec![login(), ("LIST ", format!("{folders}$TAG OK listed\r\n"))],
        );
        let error = fetch_sent_blocking(&mailbox(address), None, 0, 200).unwrap_err();
        assert!(error.to_string().contains("configure imap.sent_folder"));
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[test]
fn configured_sent_folder_and_completed_cursor_skip_unneeded_commands() {
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            (
                "EXAMINE ",
                "* 0 EXISTS\r\n* OK [UIDVALIDITY 55] valid\r\n$TAG OK selected\r\n".to_string(),
            ),
            logout(),
        ],
    );
    let mut mailbox = mailbox(address);
    mailbox.imap.sent_folder = Some("Custom Sent".to_string());
    let cursor = SentSyncCursor {
        folder_name: "Custom Sent".to_string(),
        uid_validity: 55,
        last_uid: u32::MAX as u64,
        backfill_cutoff: 0,
    };
    let batch = fetch_sent_blocking(&mailbox, Some(&cursor), 0, 200).unwrap();
    assert!(batch.messages.is_empty());
    assert!(batch.complete);
    assert_eq!(server.join().unwrap().len(), 3);
}

#[test]
fn mark_seen_consumes_tagged_store_errors() {
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            select(),
            (
                "UID STORE 7 ",
                "* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n$TAG NO store rejected\r\n".to_string(),
            ),
        ],
    );
    let error = mark_seen_blocking(&mailbox(address), 7).unwrap_err();
    assert!(error.to_string().contains("store rejected"));
    assert!(server.join().unwrap()[2].contains("+FLAGS (\\Seen)"));
}

#[test]
fn malformed_greeting_and_interrupted_fetch_return_errors() {
    for greeting in ["* BYE unavailable\r\n", "malformed greeting\r\n"] {
        let (address, server) = server(greeting, vec![]);
        assert!(fetch_unseen_blocking(&mailbox(address), 10).is_err());
        assert!(server.join().unwrap().is_empty());
    }
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            select(),
            (
                "UID SEARCH UNSEEN",
                "* SEARCH 7\r\n$TAG OK searched\r\n".to_string(),
            ),
            (
                "UID FETCH 7 ",
                "* 1 FETCH (UID 7 BODY[] {1000}\r\nincomplete".to_string(),
            ),
        ],
    );
    assert!(fetch_unseen_blocking(&mailbox(address), 10).is_err());
    server.join().unwrap();
}

#[test]
fn worker_diagnoses_denied_malformed_and_interrupted_imap_without_credentials() {
    for (greeting, steps, run_id, cause) in [
        (
            "* OK ready\r\n",
            vec![(
                "LOGIN ",
                "$TAG NO authentication denied synthetic-imap-password\r\n".to_string(),
            )],
            "offline-imap-denied",
            "authentication denied",
        ),
        (
            "malformed synthetic-imap-password greeting\r\n",
            vec![],
            "offline-imap-malformed",
            "invalid IMAP response",
        ),
        (
            "* OK ready\r\n",
            vec![
                login(),
                select(),
                (
                    "UID SEARCH UNSEEN",
                    "* SEARCH 7\r\n$TAG OK searched\r\n".to_string(),
                ),
                (
                    "UID FETCH 7 ",
                    "* 1 FETCH (UID 7 BODY[] {1000}\r\nincomplete".to_string(),
                ),
            ],
            "offline-imap-interrupted",
            "UnexpectedEof",
        ),
    ] {
        let (address, server) = server(greeting, steps);
        let mut config = config();
        config.mailboxes = vec![mailbox(address)];
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (_, events) = capture(|| {
            runtime.block_on(crate::worker::run_once_with_store(
                &config,
                &StdoutLogger,
                run_id,
                &LiveMailTransport::default(),
                &LiveDecisionEngine::default(),
                &MemoryProcessingStore::default(),
            ))
        });
        let text = serde_json::to_string(&events).unwrap();
        for required in [
            run_id,
            "imap_fetch",
            "started",
            "failed",
            cause,
            "support",
            "folder=INBOX",
            "tls=false",
            "duration_ms",
        ] {
            assert!(
                text.contains(required),
                "missing diagnostic field: {required}"
            );
        }
        assert!(!text.contains("synthetic-imap-password"));
        assert!(events.iter().any(|event| event["level"] == "ERROR"));
        assert!(events.iter().all(|event| event["level"] != "DEBUG"));
        server.join().unwrap();
    }
}

#[test]
fn async_transport_keeps_the_executor_responsive_during_blocking_io() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mailbox = mailbox(listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        accepted_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        stream.write_all(b"* BYE test complete\r\n").unwrap();
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let fetch = tokio::spawn(async move {
            LiveMailTransport::default()
                .fetch_unseen(&mailbox, 10)
                .await
        });
        tokio::task::spawn_blocking(move || {
            accepted_rx.recv_timeout(Duration::from_secs(5)).unwrap()
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(1), tokio::task::yield_now())
            .await
            .unwrap();
        release_tx.send(()).unwrap();
        assert!(fetch.await.unwrap().is_err());
    });
    server.join().unwrap();
}

#[test]
fn list_search_and_fetch_reject_tagged_failure_even_after_partial_data() {
    for status in ["NO", "BAD"] {
        let (address, server) = server(
            "* OK ready\r\n",
            vec![
                login(),
                (
                    "LIST ",
                    format!("* LIST (\\Sent) \"/\" \"Sent\"\r\n$TAG {status} list rejected\r\n"),
                ),
            ],
        );
        assert!(fetch_sent_blocking(&mailbox(address), None, 0, 200)
            .unwrap_err()
            .to_string()
            .contains("list rejected"));
        server.join().unwrap();

        let (address, server) = self::server(
            "* OK ready\r\n",
            vec![
                login(),
                select(),
                (
                    "UID SEARCH UNSEEN",
                    format!("* SEARCH 7\r\n$TAG {status} search rejected\r\n"),
                ),
            ],
        );
        assert!(fetch_unseen_blocking(&mailbox(address), 10)
            .unwrap_err()
            .to_string()
            .contains("search rejected"));
        server.join().unwrap();

        let fetch = fetch_response(false)
            .replace("$TAG OK fetched", &format!("$TAG {status} fetch rejected"));
        let (address, server) = self::server(
            "* OK ready\r\n",
            vec![
                login(),
                select(),
                (
                    "UID SEARCH UNSEEN",
                    "* SEARCH 7\r\n$TAG OK searched\r\n".to_string(),
                ),
                ("UID FETCH 7 ", fetch),
            ],
        );
        assert!(fetch_unseen_blocking(&mailbox(address), 10)
            .unwrap_err()
            .to_string()
            .contains("fetch rejected"));
        server.join().unwrap();
    }
}

#[test]
fn fetch_rejects_eof_after_complete_body_without_tagged_completion() {
    let fetch = fetch_response(false).replace("$TAG OK fetched\r\n", "");
    let (address, server) = server(
        "* OK ready\r\n",
        vec![
            login(),
            select(),
            (
                "UID SEARCH UNSEEN",
                "* SEARCH 7\r\n$TAG OK searched\r\n".to_string(),
            ),
            ("UID FETCH 7 ", fetch),
        ],
    );
    assert!(fetch_unseen_blocking(&mailbox(address), 10)
        .unwrap_err()
        .to_string()
        .contains("ended before tagged completion"));
    server.join().unwrap();
}

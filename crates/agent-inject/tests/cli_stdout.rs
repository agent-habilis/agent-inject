//! The binary's output contract: stdout carries the URL, one line per saved
//! file, then a done line when the sender finishes; the process then exits 0
//! on its own, and stderr stays empty unless something fails.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use agent_inject::test_support::{finish, loopback_sender, upload};
use agent_inject_proto::{Accept, InjectTicket, RequestHeader, Status, UPLOAD_ALPN};

fn line(reader: &mut impl BufRead) -> serde_json::Value {
    let mut text = String::new();
    reader.read_line(&mut text).expect("read stdout");
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
}

#[tokio::test]
async fn json_stdout_is_the_url_one_path_per_file_then_done() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-inject"))
        .args(["--loopback", "--output", "json"])
        .arg(dir.path().join("inbox"))
        .env("AGENT_INJECT_WEB_ORIGIN", "https://example.test/")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent-inject");
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let url = line(&mut stdout)["url"].as_str().unwrap().to_owned();
    let ticket = url
        .strip_prefix("https://example.test/app/inject/")
        .unwrap_or_else(|| panic!("unexpected url {url}"));
    let ticket = InjectTicket::decode(ticket).unwrap();

    let endpoint = loopback_sender().await.unwrap();
    let conn = endpoint
        .connect(ticket.addr.clone(), UPLOAD_ALPN)
        .await
        .unwrap();
    for (index, name) in ["one.txt", "two.txt"].into_iter().enumerate() {
        let header = RequestHeader {
            secret: ticket.secret,
            upload_id: [u8::try_from(index).unwrap(); 16],
            name: name.to_owned(),
            size: 3,
        };
        let response = upload(&conn, &header, b"abc").await.unwrap();
        assert_eq!(response.status, Status::Ok);
        let path = line(&mut stdout)["path"].as_str().unwrap().to_owned();
        assert!(path.ends_with(&format!("inbox/{name}")), "{path}");
        assert!(std::path::Path::new(&path).is_absolute());
    }

    // The sender's done signal, not ctrl-c, ends the process. Wait for the
    // exit first, so a binary that ignores the signal fails here instead of
    // hanging on a stdout line that never comes.
    assert_eq!(
        finish(&conn, &ticket.secret).await.unwrap().status,
        Status::Ok
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let exit = loop {
        if let Some(exit) = child.try_wait().unwrap() {
            break exit;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let mut rest = String::new();
            let _ = stdout.read_to_string(&mut rest);
            panic!("agent-inject did not exit after the done signal; stdout after it: {rest:?}");
        }
        // Async, so this test's own endpoint keeps running and can
        // acknowledge the answer to the done signal.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(exit.success(), "{exit:?}");

    let done = line(&mut stdout);
    let files: Vec<&str> = done["done"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file.as_str().unwrap())
        .collect();
    assert_eq!(files.len(), 2);
    assert!(files[0].ends_with("inbox/one.txt") && files[1].ends_with("inbox/two.txt"));
    assert!(done["done"]["dir"].as_str().unwrap().ends_with("inbox"));

    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert!(rest.is_empty(), "unexpected stdout: {rest:?}");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert!(stderr.is_empty(), "stderr is for errors only: {stderr:?}");
}

#[tokio::test]
async fn accept_images_puts_the_mode_in_the_ticket_and_refuses_other_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-inject"))
        .args(["--loopback", "--output", "json", "--accept", "images"])
        .arg(dir.path())
        .env("AGENT_INJECT_WEB_ORIGIN", "https://example.test")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn agent-inject");
    let start = line(&mut BufReader::new(child.stdout.take().unwrap()));
    let url = start["url"].as_str().unwrap();
    assert!(
        start["qr"].as_str().is_some_and(|qr| !qr.is_empty()),
        "the agent prints this QR for the user: {start}"
    );
    let ticket = InjectTicket::decode(url.rsplit('/').next().unwrap()).unwrap();
    assert_eq!(ticket.accept, Accept::Images);

    let endpoint = loopback_sender().await.unwrap();
    let conn = endpoint
        .connect(ticket.addr.clone(), UPLOAD_ALPN)
        .await
        .unwrap();
    let header = RequestHeader {
        secret: ticket.secret,
        upload_id: [1; 16],
        name: "notes.pdf".to_owned(),
        size: 3,
    };
    let response = upload(&conn, &header, b"abc").await.unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(response.status, Status::NotAccepted);
}

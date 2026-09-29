//! The binary's output contract: stdout carries the URL and one line per
//! saved file, stderr stays empty unless something fails, and ctrl-c exits
//! cleanly.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

use agent_inject::test_support::{loopback_sender, upload};
use agent_inject_proto::{InjectTicket, RequestHeader, Status, UPLOAD_ALPN};

fn line(reader: &mut impl BufRead) -> serde_json::Value {
    let mut text = String::new();
    reader.read_line(&mut text).expect("read stdout");
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
}

#[tokio::test]
async fn json_stdout_is_the_url_then_one_path_per_file() {
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

    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    let exit = child.wait().unwrap();
    assert!(exit.success(), "{exit:?}");

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

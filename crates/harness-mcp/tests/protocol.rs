//! A scripted stdin session through the real binary (docs/MCP-DESIGN.md
//! §5 "Protocol"): every stdout line is one JSON-RPC 2.0 message, and the
//! session ends cleanly on EOF.

mod common;

use common::*;
use serde_json::{json, Value};

#[test]
fn a_scripted_session_over_stdio() {
    let zopfli = repo().join("targets/zopfli");
    let roots = repo().join("targets/tractor/cases");
    let mut c = Client::start(&[
        "--target",
        zopfli.to_str().unwrap(),
        "--target-root",
        roots.to_str().unwrap(),
        "--harness",
        "/bin/sh",
    ]);
    // Any requested version → 2025-06-18.
    c.request(
        json!(1),
        "initialize",
        json!({"protocolVersion": "2024-11-05", "capabilities": {},
               "clientInfo": {"name": "t", "version": "0"}}),
    );
    let (init, _) = c.response(&json!(1), 30);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(
        init["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "harness-mcp");
    // A notification gets no reply: the next message answers the ping.
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    c.request(json!("p"), "ping", json!({}));
    let (ping, before) = c.response(&json!("p"), 10);
    assert!(before.is_empty(), "{before:?}");
    assert_eq!(ping["result"], json!({}));

    c.request(json!(2), "tools/list", json!({}));
    let (list, _) = c.response(&json!(2), 10);
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert_eq!(t["outputSchema"]["type"], "object", "{t}");
    }

    let code = |v: &Value| v["error"]["code"].as_i64().unwrap();
    c.request(json!(3), "resources/list", json!({}));
    assert_eq!(code(&c.response(&json!(3), 10).0), -32601);
    c.call(4, "harness_verify", json!({}), None);
    assert_eq!(code(&c.response(&json!(4), 10).0), -32602);
    c.call(5, "harness_unit", json!({"unit": "u", "bogus": true}), None);
    assert_eq!(code(&c.response(&json!(5), 10).0), -32602);
    c.call(6, "harness_promote", json!({"unit": "u"}), None);
    assert_eq!(code(&c.response(&json!(6), 10).0), -32602);

    // Malformed JSON, a batch, a line over 1 MiB: errors with `id: null`.
    c.raw("{\"jsonrpc\":\"2.0\",");
    c.raw(r#"[{"jsonrpc":"2.0","id":7,"method":"ping"}]"#);
    c.raw(&format!(
        r#"{{"jsonrpc":"2.0","id":8,"method":"ping","params":{{"pad":"{}"}}}}"#,
        "x".repeat(1024 * 1024)
    ));
    c.request(json!(9), "ping", json!({}));
    let (_, errors) = c.response(&json!(9), 10);
    let got: Vec<(i64, Value)> = errors.iter().map(|e| (code(e), e["id"].clone())).collect();
    assert_eq!(
        got,
        vec![
            (-32700, Value::Null),
            (-32600, Value::Null),
            (-32600, Value::Null)
        ]
    );

    // A read tool answers with both channels.
    c.call(10, "harness_status", json!({}), None);
    let (status, _) = c.response(&json!(10), 60);
    assert!(!is_error(&status));
    assert_eq!(
        structured(&status)["units"][0]["id"]["text"],
        "u001-katajainen"
    );
    // A target outside every root is refused before anything is read.
    c.call(11, "harness_status", json!({"target": "/"}), None);
    let (refused, _) = c.response(&json!(11), 10);
    assert!(is_error(&refused));
    assert_eq!(structured(&refused)["error"]["kind"], "target");

    // EOF: a clean exit.
    c.close_stdin();
    let status = c.wait_exit(10);
    assert_eq!(status.code(), Some(0));
    assert!(c.drain(200).is_empty());
}

#[test]
fn a_bad_command_line_exits_2_and_writes_nothing_to_stdout() {
    let out = std::process::Command::new(server_bin())
        .args(["--target", "/no/such/dir"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--target"));
}

/// A fake `harness`: its act prints a turn-start (after its INT trap is
/// set), then spins until interrupted, when it writes `$0.log`.
fn spinner(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("harness-mcp-sig-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("harness");
    std::fs::write(
        &path,
        r#"#!/bin/sh
trap 'echo interrupted > "$0.log"; exit 130' INT
echo '{"k":"turn-start","unit":"u","attempt":"a-000000000001","index":1,"kind":"steer","request_key":"k"}'
while :; do sleep 0.05; done
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn start_spinning(tag: &str) -> (Client, std::path::PathBuf) {
    let fake = spinner(tag);
    let zopfli = repo().join("targets/zopfli");
    let mut c = Client::start(&[
        "--target",
        zopfli.to_str().unwrap(),
        "--harness",
        fake.to_str().unwrap(),
    ]);
    c.initialize();
    c.call(
        1,
        "harness_promote",
        json!({"unit": "u001-katajainen", "attempt": "a-000000000001"}),
        Some("p"),
    );
    // The first progress: the act runs, past its trap.
    let seen = c.drain(3000);
    assert!(
        seen.iter().any(|m| m["method"] == "notifications/progress"),
        "{seen:?}"
    );
    (c, fake)
}

/// §2 "Shutdown", §R2 TESTS-7: on SIGTERM, SIGINT or SIGHUP the server
/// interrupts the running act's process group, then dies BY the signal.
#[test]
fn a_signal_interrupts_the_act_and_the_server_dies_by_it() {
    use std::os::unix::process::ExitStatusExt;
    for (sig, name) in [(15, "TERM"), (2, "INT"), (1, "HUP")] {
        let (mut c, fake) = start_spinning(name);
        let server = c.pid();
        let children = children_of(server);
        assert_eq!(children.len(), 1, "{children:?}");
        assert!(std::process::Command::new("/bin/kill")
            .args([format!("-{name}"), server.to_string()])
            .status()
            .unwrap()
            .success());
        let status = c.wait_exit(10);
        assert_eq!(status.signal(), Some(sig), "{name}: {status:?}");
        let log = std::fs::read_to_string(format!("{}.log", fake.display()))
            .unwrap_or_else(|_| panic!("{name}: the act was not interrupted"));
        assert_eq!(log.trim(), "interrupted");
        wait_for("the act to end", 10, || (!alive(children[0])).then_some(()));
        let _ = std::fs::remove_dir_all(fake.parent().unwrap());
    }
}

/// §2 "Shutdown": stdin EOF interrupts the running act and exits 0.
#[test]
fn eof_interrupts_the_act_and_exits_0() {
    let (mut c, fake) = start_spinning("eof");
    let children = children_of(c.pid());
    c.close_stdin();
    let status = c.wait_exit(10);
    assert_eq!(status.code(), Some(0));
    let log = std::fs::read_to_string(format!("{}.log", fake.display())).unwrap();
    assert_eq!(log.trim(), "interrupted");
    for child in children {
        wait_for("the act to end", 10, || (!alive(child)).then_some(()));
    }
    let _ = std::fs::remove_dir_all(fake.parent().unwrap());
}

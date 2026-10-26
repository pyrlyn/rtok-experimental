// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! T401 Check: a fake client drives `rtok mcp --http` over loopback (initialize, tools/list,
//! one `read` call), the bearer token and the Origin/Host checks refuse what they must, and the
//! `read` call leaves the same measurement rows as the same call over stdio.

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

const TOKEN: &str = "t401-test-token-0123456789";
const READ_ARGS: &str = r#"{"path":"hello.txt"}"#;

/// Temp `HOME`, `RTOK_HOME` and project dir: nothing touches the real home.
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("rtok-mcp-http-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("proj")).unwrap();
        std::fs::write(root.join("proj/hello.txt"), "hello from t401\n").unwrap();
        Self { root }
    }

    fn db(&self) -> PathBuf {
        self.root.join("home/rtok.db")
    }

    fn rtok(&self) -> Command {
        let mut cmd = Command::new(crate::it::common::bin_path("rtok"));
        cmd.current_dir(self.root.join("proj"))
            .env("HOME", self.root.join("home"))
            .env("RTOK_HOME", self.root.join("home"))
            .env("RTOK_CORE_DB_PATH", self.db())
            .env("RTOK_HOST_SANDBOX", "1")
            .env_remove("RTOK_AGENT_ID")
            .env_remove("RTOK_MCP_TOKEN");
        cmd
    }

    /// `(call plugin, surface, kind, tool, ok, token plugin, phase, source, tokens)` of every
    /// `mcp_call`, in a fixed order.
    #[allow(clippy::type_complexity)]
    fn mcp_rows(
        &self,
    ) -> Vec<(
        Option<String>,
        String,
        String,
        Option<String>,
        i32,
        Option<String>,
        String,
        String,
        i64,
    )> {
        let store = rtok::store::Store::open(&self.db()).unwrap();
        store.mcp_call_token_rows().unwrap()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Our own `rtok mcp --http` child, killed when the test ends either way.
struct Served(Child);

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start the server on an OS-picked loopback port and return the endpoint it printed.
fn serve(sandbox: &Sandbox) -> (Served, String) {
    let mut child = sandbox
        .rtok()
        .args(["mcp", "--http", "127.0.0.1:0"])
        .env("RTOK_MCP_TOKEN", TOKEN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtok mcp --http");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let served = Served(child);
    loop {
        let line = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("rtok mcp --http printed no endpoint");
        if let Some(url) = line.strip_prefix("rtok mcp: serving ") {
            assert!(!line.contains(TOKEN), "the token reached stderr: {line}");
            return (served, url.to_string());
        }
    }
}

fn post(client: &reqwest::Client, url: &str, body: &Value) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "2025-06-18")
        .body(body.to_string())
}

fn rpc(id: u32, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

#[tokio::test(flavor = "multi_thread")]
async fn http_serves_initialize_list_and_read_and_refuses_without_the_token() {
    let sandbox = Sandbox::new("e2e");
    let (_served, url) = serve(&sandbox);
    assert!(
        url.starts_with("http://127.0.0.1:") && url.ends_with("/mcp"),
        "{url}"
    );
    let client = reqwest::Client::new();
    let init = rpc(
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake", "version": "1"}}),
    );

    // Fail closed: no token, a wrong token and another scheme are all 401 before any MCP work.
    let none = post(&client, &url, &init).send().await.unwrap();
    assert_eq!(none.status(), 401);
    assert_eq!(none.headers()["www-authenticate"], "Bearer");
    for auth in [format!("Bearer {TOKEN}x"), format!("Basic {TOKEN}")] {
        let res = post(&client, &url, &init)
            .header("authorization", auth)
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 401);
    }

    let authed = |body: &Value| post(&client, &url, body).bearer_auth(TOKEN);

    // DNS rebinding guard: a browser page's Origin and a foreign Host are 403 even with the token.
    let res = authed(&init)
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);
    let res = authed(&init)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    let res = authed(&init).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let v: Value = res.text().await.unwrap().parse::<Value>().unwrap();
    assert_eq!(v["result"]["serverInfo"]["name"], "rtok", "{v}");
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18", "{v}");

    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(authed(&note).send().await.unwrap().status(), 202);

    let v: Value = authed(&rpc(2, "tools/list", json!({})))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    // `[mcp] http_tools` defaults to the read-only file tools; `expand` always stays listed.
    for name in ["read", "search", "tree", "expand"] {
        assert!(names.contains(&name), "{name} missing from {names:?}");
    }
    assert!(!names.contains(&"mem_save"), "{names:?}");
    // stdio always lists `whoami`; over HTTP it would answer as whoever started the server.
    assert!(!names.contains(&"whoami"), "{names:?}");

    let args: Value = serde_json::from_str(READ_ARGS).unwrap();
    let v: Value = authed(&rpc(
        3,
        "tools/call",
        json!({"name": "read", "arguments": args}),
    ))
    .send()
    .await
    .unwrap()
    .text()
    .await
    .unwrap()
    .parse::<Value>()
    .unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("hello from t401"), "{v}");

    // Off the HTTP allow-list means unknown, the same answer stdio gives.
    let v: Value = authed(&rpc(
        4,
        "tools/call",
        json!({"name": "mem_save", "arguments": {"body": "x"}}),
    ))
    .send()
    .await
    .unwrap()
    .text()
    .await
    .unwrap()
    .parse::<Value>()
    .unwrap();
    assert_eq!(v["result"]["isError"], true, "{v}");
    assert_eq!(v["result"]["content"][0]["text"], "unknown tool: mem_save");
}

/// The same `read` over stdio and over HTTP leaves identical rows: no saving is claimed for
/// HTTP that stdio would not record.
#[tokio::test(flavor = "multi_thread")]
async fn http_read_records_the_same_rows_as_stdio() {
    let over_http = Sandbox::new("rows-http");
    {
        let (_served, url) = serve(&over_http);
        let client = reqwest::Client::new();
        let args: Value = serde_json::from_str(READ_ARGS).unwrap();
        let v: Value = post(
            &client,
            &url,
            &rpc(1, "tools/call", json!({"name": "read", "arguments": args})),
        )
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
        assert_eq!(v["result"]["isError"], false, "{v}");
    }

    let over_stdio = Sandbox::new("rows-stdio");
    stdio_read(&over_stdio);

    let http_rows = over_http.mcp_rows();
    assert_eq!(http_rows.len(), 3, "{http_rows:?}");
    assert_eq!(http_rows, over_stdio.mcp_rows());
}

fn stdio_read(sandbox: &Sandbox) {
    let mut child = sandbox
        .rtok()
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rtok mcp");
    let call = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"read","arguments":{READ_ARGS}}}}}"#
    );
    writeln!(child.stdin.take().unwrap(), "{call}").unwrap();
    let out = child.wait_with_output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
}

#[cfg(feature = "graph")]
#[tokio::test(flavor = "multi_thread")]
async fn http_text_graph_does_not_reveal_credential_definitions() {
    let sandbox = Sandbox::new("text-graph-secrets");
    let project = sandbox.root.join("proj");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::write(
        sandbox.root.join("home/config.toml"),
        "[mcp]\nhttp_tools = [\"symbol\"]\n[plugins.graph]\nbackend = \"text\"\n",
    )
    .unwrap();
    std::fs::write(project.join("public.txt"), "const visible = 1;\n").unwrap();
    std::fs::write(project.join(".env"), "const hidden = secret_marker;\n").unwrap();
    let (_served, url) = serve(&sandbox);
    let client = reqwest::Client::new();
    for (id, name, expected) in [(1, "visible", "public.txt"), (2, "hidden", "no definition")] {
        let value: Value = post(
            &client,
            &url,
            &rpc(
                id,
                "tools/call",
                json!({"name": "symbol", "arguments": {"name": name}}),
            ),
        )
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap()
        .parse::<Value>()
        .unwrap();
        assert_eq!(value["result"]["isError"], false, "{value}");
        let text = value["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(expected), "{value}");
        assert!(!text.contains("secret_marker"), "{value}");
    }
}

#[test]
fn http_refuses_to_start_without_a_token() {
    let sandbox = Sandbox::new("no-token");
    let out = sandbox
        .rtok()
        .args(["mcp", "--http", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("RTOK_MCP_TOKEN"), "{err}");
}

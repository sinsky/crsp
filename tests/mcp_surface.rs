use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

use serde_json::Value;

#[test]
fn mcp_alias_and_canonical_return_identical_initialize_responses() {
    let request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"surface\",\"version\":\"1\"}}}\n";
    let run = |command: &str| {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_crsp"))
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(request).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let _ = child.kill();
        let _ = child.wait();
        line
    };
    let canonical: Value = serde_json::from_str(&run("start-mcp-server")).unwrap();
    let alias: Value = serde_json::from_str(&run("mcp")).unwrap();
    assert_eq!(canonical["jsonrpc"], "2.0");
    assert_eq!(canonical["id"], 1);
    assert_eq!(canonical["result"]["serverInfo"]["name"], "Crsp");
    assert_eq!(canonical["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(alias["result"]["serverInfo"]["name"], "Crsp");
    assert_eq!(canonical, alias);
}

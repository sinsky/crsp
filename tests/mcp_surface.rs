use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;

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
    assert_eq!(run("mcp"), run("start-mcp-server"));
}

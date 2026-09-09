use std::path::PathBuf;

use crsp::api::BaseUrls;
use crsp::mcp::server::{McpServer, validate_project_dir};
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ToolAnnotations};
use rmcp::service::{ClientServiceExt, ServiceExt};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{ClientHandler, RoleClient, RoleServer};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer as WireMockServer, ResponseTemplate};

#[derive(Default)]
struct TestClient;

impl ClientHandler for TestClient {}

async fn connected() -> (
    rmcp::service::RunningService<RoleClient, TestClient>,
    rmcp::service::RunningService<RoleServer, McpServer>,
) {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);
    let server = McpServer::new_for_tests();
    let server_task = tokio::spawn(async move {
        server
            .serve(AsyncRwTransport::<RoleServer, _, _>::new_server(
                server_read,
                server_write,
            ))
            .await
            .expect("server initializes")
    });
    let client = TestClient
        .serve_with_lifecycle(
            AsyncRwTransport::<RoleClient, _, _>::new_client(client_read, client_write),
            rmcp::service::ClientLifecycleMode::Initialize,
        )
        .await
        .expect("client initializes");
    let server = server_task.await.expect("server task completes");
    (client, server)
}

fn args(values: Value) -> Option<serde_json::Map<String, Value>> {
    serde_json::from_value(values).unwrap()
}

fn result_value(result: CallToolResult) -> CallToolResult {
    result
}

#[tokio::test]
async fn advertises_exact_tools_schemas_annotations_and_metadata() {
    let (client, server) = connected().await;
    let info = client.peer_info().expect("server info");
    let server_info = info.server_info.as_ref().expect("server implementation");
    assert_eq!(server_info.name, "Crsp");
    assert_eq!(server_info.version, env!("CARGO_PKG_VERSION"));

    let listed = client.peer().list_tools(None).await.expect("tools/list");
    let mut names: Vec<_> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "clone_project",
            "create_project",
            "list_projects",
            "pull_files",
            "push_files"
        ]
    );

    for tool in &listed.tools {
        let annotation = tool.annotations.as_ref().expect("annotations");
        assert_eq!(annotation.open_world_hint, Some(false));
        assert_eq!(annotation.idempotent_hint, Some(false));
        if tool.name == "list_projects" {
            assert_eq!(annotation.destructive_hint, Some(false));
            assert_eq!(annotation.read_only_hint, Some(true));
        } else {
            assert_eq!(annotation.destructive_hint, Some(true));
            assert_eq!(annotation.read_only_hint, Some(false));
        }
    }

    let push = listed
        .tools
        .iter()
        .find(|tool| tool.name == "push_files")
        .unwrap();
    assert_eq!(
        push.input_schema.get("required"),
        Some(&json!(["projectDir"]))
    );
    assert_eq!(
        push.input_schema["properties"]["projectDir"]["type"],
        "string"
    );
    assert!(push.output_schema.is_some());
    server.cancel().await.expect("close server");
}

#[tokio::test]
async fn clone_without_script_id_returns_exact_error() {
    let (client, server) = connected().await;
    let project = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let response = client
        .call_tool_once(
            CallToolRequestParams::new("clone_project")
                .with_arguments(args(json!({"projectDir": project.path()})).unwrap()),
        )
        .await
        .expect("tools/call");
    let result = match response {
        CallToolResponse::Complete(result) => result,
        _ => panic!("expected result"),
    };
    assert_eq!(result.is_error, Some(true));
    assert_eq!(
        result.content[0].as_text().unwrap().text,
        "Script ID is required."
    );
    server.cancel().await.expect("close server");
}

#[tokio::test]
async fn list_projects_returns_exact_text_and_structured_content() {
    let api = WireMockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                {"id": "script-1", "name": "Alpha"},
                {"id": "script-2", "name": "Beta"}
            ]
        })))
        .mount(&api)
        .await;
    let urls = BaseUrls {
        drive: api.uri(),
        ..BaseUrls::default()
    };
    let server_impl = McpServer::with_base_urls(urls);
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);
    let server_task = tokio::spawn(async move {
        server_impl
            .serve(AsyncRwTransport::<RoleServer, _, _>::new_server(
                server_read,
                server_write,
            ))
            .await
            .unwrap()
    });
    let client = TestClient
        .serve(AsyncRwTransport::<RoleClient, _, _>::new_client(
            client_read,
            client_write,
        ))
        .await
        .unwrap();
    let response = client
        .call_tool_once(
            CallToolRequestParams::new("list_projects")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    let result = match response {
        CallToolResponse::Complete(result) => result,
        _ => panic!("expected complete result"),
    };
    assert_eq!(
        result.content[0].as_text().unwrap().text,
        "Found 2 Apps Script projects."
    );
    assert_eq!(
        result.content[1].as_text().unwrap().text,
        "Alpha (script-1)"
    );
    assert_eq!(
        result.structured_content,
        Some(
            json!({"scripts": [{"scriptId": "script-1", "name": "Alpha"}, {"scriptId": "script-2", "name": "Beta"}]})
        )
    );
    server_task.await.unwrap().cancel().await.unwrap();
}

#[test]
fn validates_project_and_source_jails() {
    let home = PathBuf::from("/home/test");
    let cwd = home.join("work");
    let outside = PathBuf::from("/etc/crsp");
    let error = validate_project_dir(&outside, &home, &cwd).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Security Error: projectDir must be within the user home directory or current working directory. Resolved path \"/etc/crsp\" is not permitted."
    );
    assert!(crsp::mcp::server::validate_source_dir(&cwd, "../escape").is_err());
    assert!(crsp::mcp::server::validate_source_dir(&cwd, "src").is_ok());
}

#[test]
fn tool_annotations_are_expressible_by_rmcp() {
    let annotations = ToolAnnotations::new()
        .open_world(false)
        .destructive(false)
        .idempotent(false)
        .read_only(true);
    assert_eq!(annotations.read_only_hint, Some(true));
}

#[allow(dead_code)]
fn _keep_types_used(_: Value, _: PathBuf) {
    let _ = result_value;
}

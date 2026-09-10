use std::path::PathBuf;

use crsp::api::BaseUrls;
use crsp::mcp::server::{McpServer, validate_project_dir};
use rmcp::model::{CallToolRequestParams, CallToolResponse, ToolAnnotations};
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

    for tool in &listed.tools {
        let required = tool
            .input_schema
            .get("required")
            .cloned()
            .unwrap_or(Value::Null);
        let properties = tool
            .input_schema
            .get("properties")
            .cloned()
            .unwrap_or(json!({}));
        match tool.name.as_ref() {
            "push_files" | "pull_files" => {
                assert_eq!(required, json!(["projectDir"]));
                assert_eq!(properties["projectDir"]["type"], "string");
            }
            "create_project" => {
                assert_eq!(required, json!(["projectDir"]));
                assert_eq!(properties["projectDir"]["type"], "string");
                assert!(
                    properties["sourceDir"]["type"] == "string"
                        || properties["sourceDir"]["type"] == json!(["string", "null"])
                );
                assert!(
                    properties["projectName"]["type"] == "string"
                        || properties["projectName"]["type"] == json!(["string", "null"])
                );
            }
            "clone_project" => {
                assert_eq!(required, json!(["projectDir"]));
                assert_eq!(properties["projectDir"]["type"], "string");
                assert!(
                    properties["sourceDir"]["type"] == "string"
                        || properties["sourceDir"]["type"] == json!(["string", "null"])
                );
                assert!(
                    properties["scriptId"]["type"] == "string"
                        || properties["scriptId"]["type"] == json!(["string", "null"])
                );
            }
            "list_projects" => {
                assert!(required.is_null());
                assert!(
                    properties
                        .as_object()
                        .is_none_or(|properties| properties.is_empty())
                );
            }
            name => panic!("unexpected tool {name}"),
        }
        if tool.name != "list_projects" {
            assert!(tool.output_schema.is_some());
        }
    }
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

async fn connected_with_urls(
    urls: BaseUrls,
) -> (
    rmcp::service::RunningService<RoleClient, TestClient>,
    rmcp::service::RunningService<RoleServer, McpServer>,
) {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);
    let server = McpServer::with_base_urls(urls);
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

async fn call_complete(
    client: &rmcp::service::RunningService<RoleClient, TestClient>,
    name: &str,
    arguments: Value,
) -> rmcp::model::CallToolResult {
    match client
        .call_tool_once(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().unwrap().clone()),
        )
        .await
        .expect("tools/call")
    {
        CallToolResponse::Complete(result) => result,
        _ => panic!("expected complete result"),
    }
}

fn text(result: &rmcp::model::CallToolResult, index: usize) -> &str {
    result.content[index].as_text().unwrap().text.as_str()
}

fn all_test_urls(base: &str) -> BaseUrls {
    BaseUrls {
        script: base.to_string(),
        drive: base.to_string(),
        ..BaseUrls::default()
    }
}

#[tokio::test]
async fn push_and_pull_return_exact_success_shapes_without_status() {
    let api = WireMockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script-1/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"files": [{"name": "Code", "type": "SERVER_JS", "source": "old"}]}),
        ))
        .mount(&api)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/projects/script-1/content"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&api)
        .await;
    let project = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(
        project.path().join(".clasp.json"),
        r#"{"scriptId":"script-1"}"#,
    )
    .unwrap();
    std::fs::write(project.path().join("Code.js"), "new").unwrap();
    let (client, server) = connected_with_urls(all_test_urls(&api.uri())).await;
    let push = call_complete(&client, "push_files", json!({"projectDir": project.path()})).await;
    assert_eq!(
        text(&push, 0),
        format!(
            "Pushed project in {} to remote server successfully.",
            project.path().display()
        )
    );
    assert_eq!(
        text(&push, 1),
        format!("Updated file: {}", project.path().join("Code.js").display())
    );
    assert_eq!(
        push.structured_content,
        Some(
            json!({"scriptId":"script-1","projectDir":project.path(),"files":[project.path().join("Code.js")]})
        )
    );
    assert!(
        push.structured_content
            .as_ref()
            .unwrap()
            .get("status")
            .is_none()
    );
    let pull = call_complete(&client, "pull_files", json!({"projectDir": project.path()})).await;
    assert_eq!(
        text(&pull, 0),
        format!(
            "Pulled project in {} to local filesystem successfully.",
            project.path().display()
        )
    );
    assert_eq!(
        text(&pull, 1),
        format!("Updated file: {}", project.path().join("Code.js").display())
    );
    assert_eq!(
        pull.structured_content,
        Some(
            json!({"scriptId":"script-1","projectDir":project.path(),"files":[project.path().join("Code.js")]})
        )
    );
    server.cancel().await.unwrap();
}

#[tokio::test]
async fn pull_failure_uses_correct_error_prefix() {
    let api = WireMockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/script-1/content"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"error":{"message":"boom"}})))
        .mount(&api)
        .await;
    let project = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(
        project.path().join(".clasp.json"),
        r#"{"scriptId":"script-1"}"#,
    )
    .unwrap();
    let (client, server) = connected_with_urls(all_test_urls(&api.uri())).await;
    let result = call_complete(&client, "pull_files", json!({"projectDir": project.path()})).await;
    assert!(text(&result, 0).starts_with("Error pulling project:"));
    server.cancel().await.unwrap();
}

#[tokio::test]
async fn create_project_runs_create_pull_and_update_settings_flow() {
    let api = WireMockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"scriptId":"created-1"})))
        .mount(&api)
        .await;
    Mock::given(method("GET")).and(path("/v1/projects/created-1/content"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"files":[{"name":"appsscript","type":"JSON","source":"{}"},{"name":"Code","type":"SERVER_JS","source":"function main() {}"}]}))).mount(&api).await;
    let project = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap()
        .keep();
    let (client, server) = connected_with_urls(all_test_urls(&api.uri())).await;
    let result = call_complete(
        &client,
        "create_project",
        json!({"projectDir":project,"projectName":"Demo"}),
    )
    .await;
    assert_eq!(
        text(&result, 0),
        format!(
            "Created project created-1 in {} successfully.",
            project.display()
        )
    );
    assert_eq!(
        result.structured_content.as_ref().unwrap()["scriptId"],
        "created-1"
    );
    assert!(project.join(".clasp.json").exists());
    assert!(project.join("appsscript.json").exists());
    assert!(project.join("Code.js").exists());
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(project.join(".clasp.json")).unwrap()
        )
        .unwrap()["scriptId"],
        "created-1"
    );
    server.cancel().await.unwrap();
}

#[tokio::test]
async fn clone_success_writes_settings_and_is_stateless_per_server() {
    let api = WireMockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/projects/clone-1/content"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"files":[{"name":"Code","type":"SERVER_JS","source":"clone"}]}),
            ),
        )
        .expect(2)
        .mount(&api)
        .await;
    let first = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let second = tempfile::Builder::new()
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let (client, server) = connected_with_urls(all_test_urls(&api.uri())).await;
    for project in [first.path(), second.path()] {
        let result = call_complete(
            &client,
            "clone_project",
            json!({"projectDir":project,"scriptId":"clone-1"}),
        )
        .await;
        assert_eq!(
            text(&result, 0),
            format!(
                "Cloned project clone-1 in {} successfully.",
                project.display()
            )
        );
        assert!(project.join(".clasp.json").exists());
    }
    server.cancel().await.unwrap();
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

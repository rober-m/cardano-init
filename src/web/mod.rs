use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;

use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};
use crate::scaffold::planner;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("failed to bind to port {port}: {source}")]
    Bind { port: u16, source: std::io::Error },
}

// ---------------------------------------------------------------------------
// JSON response types
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
struct RegistryResponse {
    tools: Vec<ToolResponse>,
}

#[derive(serde::Serialize)]
struct ToolResponse {
    id: String,
    name: String,
    description: String,
    website: String,
    languages: Vec<String>,
    roles: Vec<String>,
}

#[derive(serde::Serialize)]
struct PlanResponse {
    files: Vec<String>,
}

#[derive(serde::Serialize)]
struct ErrorResponse {
    error: String,
}

// ---------------------------------------------------------------------------
// Embedded UI
// ---------------------------------------------------------------------------

const INDEX_HTML: &str = include_str!("ui.html");

// ---------------------------------------------------------------------------
// Request parsing
// ---------------------------------------------------------------------------

struct Request {
    path: String,
    query: HashMap<String, String>,
}

fn parse_request(reader: &mut BufReader<&std::net::TcpStream>) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;

    // e.g. "GET /api/plan?on_chain=aiken&nix=true HTTP/1.1"
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let uri = parts[1];
    let (path, query_str) = match uri.split_once('?') {
        Some((p, q)) => (p, q),
        None => (uri, ""),
    };

    let query = parse_query(query_str);

    Some(Request {
        path: path.to_string(),
        query,
    })
}

fn parse_query(qs: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if qs.is_empty() {
        return map;
    }
    for pair in qs.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(percent_decode(k), percent_decode(v));
        }
    }
    map
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn respond_html(stream: &mut std::net::TcpStream, body: &str) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn respond_json(stream: &mut std::net::TcpStream, body: &str) {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn respond_json_error(stream: &mut std::net::TcpStream, status: u16, message: &str) {
    let body = serde_json::to_string(&ErrorResponse {
        error: message.to_string(),
    })
    .unwrap_or_else(|_| r#"{"error":"internal error"}"#.to_string());

    let header = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn respond_404(stream: &mut std::net::TcpStream) {
    let body = "Not Found";
    let header = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

// ---------------------------------------------------------------------------
// Registry → JSON
// ---------------------------------------------------------------------------

fn build_registry_json(registry: &Registry) -> String {
    let tools: Vec<ToolResponse> = registry
        .all_tools()
        .iter()
        .map(|tool| {
            let mut roles: Vec<String> = tool
                .roles
                .keys()
                .map(|r| r.as_kebab().to_string())
                .collect();
            roles.sort();

            ToolResponse {
                id: tool.id.clone(),
                name: tool.name.clone(),
                description: tool.description.clone(),
                website: tool.website.clone(),
                languages: tool.languages.clone(),
                roles,
            }
        })
        .collect();

    serde_json::to_string(&RegistryResponse { tools }).expect("registry serialization cannot fail")
}

// ---------------------------------------------------------------------------
// Plan → JSON
// ---------------------------------------------------------------------------

fn build_plan_json(query: &HashMap<String, String>, registry: &Registry) -> Result<String, String> {
    let mut assignments = Vec::new();

    if let Some(tool_id) = query.get("on_chain")
        && !tool_id.is_empty()
    {
        assignments.push(RoleAssignment {
            role: Role::OnChain,
            tool_id: tool_id.clone(),
        });
    }
    if let Some(tool_id) = query.get("off_chain")
        && !tool_id.is_empty()
    {
        assignments.push(RoleAssignment {
            role: Role::OffChain,
            tool_id: tool_id.clone(),
        });
    }
    if let Some(infra_str) = query.get("infra") {
        for tool_id in infra_str.split(',') {
            let tool_id = tool_id.trim();
            if !tool_id.is_empty() {
                assignments.push(RoleAssignment {
                    role: Role::Infrastructure,
                    tool_id: tool_id.to_string(),
                });
            }
        }
    }
    if let Some(tool_id) = query.get("testing")
        && !tool_id.is_empty()
    {
        assignments.push(RoleAssignment {
            role: Role::Testing,
            tool_id: tool_id.clone(),
        });
    }

    if assignments.is_empty() {
        return Ok(serde_json::to_string(&PlanResponse { files: vec![] }).unwrap());
    }

    let nix = query.get("nix").is_some_and(|v| v == "true" || v == "1");

    let network = query
        .get("network")
        .and_then(|n| Network::from_str(n).ok())
        .unwrap_or(Network::Preview);

    let selection = Selection {
        project_name: query
            .get("name")
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| "my-project".to_string()),
        assignments,
        network,
        nix,
    };

    let plan = planner::plan(&selection, registry).map_err(|e| e.to_string())?;

    let files: Vec<String> = plan
        .entries
        .iter()
        .map(|e| e.dest.to_string_lossy().to_string())
        .collect();

    serde_json::to_string(&PlanResponse { files }).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Open browser (best-effort)
// ---------------------------------------------------------------------------

fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "start";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return;

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let _ = std::process::Command::new(cmd)
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Start the web UI server on the given port.
pub fn serve(registry: &Registry, port: u16) -> Result<(), WebError> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).map_err(|e| WebError::Bind { port, source: e })?;

    let url = format!("http://{addr}");
    println!(
        "\n  {} serving at {}\n  Press Ctrl+C to stop.\n",
        console::style("cardano-init web").bold(),
        console::style(&url).cyan().underlined(),
    );

    // Pre-build registry JSON once (served on every /api/registry request)
    let registry_json = Arc::new(build_registry_json(registry));

    open_browser(&url);

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };

        let registry_json = Arc::clone(&registry_json);

        std::thread::spawn(move || {
            let mut reader = BufReader::new(&stream);

            let req = match parse_request(&mut reader) {
                Some(r) => r,
                None => return,
            };

            drop(reader);

            match req.path.as_str() {
                "/" => respond_html(&mut stream, INDEX_HTML),
                "/api/registry" => respond_json(&mut stream, &registry_json),
                "/api/plan" => {
                    // Reload registry per request — it's embedded (no disk I/O),
                    // so this is cheap and avoids Send/Sync concerns.
                    match Registry::load() {
                        Ok(reg) => match build_plan_json(&req.query, &reg) {
                            Ok(json) => respond_json(&mut stream, &json),
                            Err(msg) => respond_json_error(&mut stream, 400, &msg),
                        },
                        Err(e) => {
                            respond_json_error(&mut stream, 500, &e.to_string());
                        }
                    }
                }
                _ => respond_404(&mut stream),
            }

            let _ = stream.flush();
        });
    }

    Ok(())
}

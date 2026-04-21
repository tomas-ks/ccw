use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, UNIX_EPOCH},
};

use cc_w_backend::{GeometryBackend, available_demo_resources};
use cc_w_platform_web::{
    WebPreparedGeometryPackage, WebPreparedPackageResponse, WebResourceCatalog,
};
use cc_w_velr::{
    IfcArtifactLayout, VelrIfcModel, available_ifc_body_resources, default_ifc_artifacts_root,
    parse_ifc_body_resource,
};
use serde::{Deserialize, Serialize};

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 8001;
const DEFAULT_ROOT: &str = "crates/cc-w-platform-web/artifacts/viewer";
const MAX_REQUEST_HEADER_BYTES: usize = 16 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
const PORT_SEARCH_LIMIT: u16 = 32;
const RESOURCES_API_PATH: &str = "/api/resources";
const PACKAGE_API_PATH: &str = "/api/package";
const CYPHER_API_PATH: &str = "/api/cypher";

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse(env::args().skip(1))?;
    let root = fs::canonicalize(&args.root)
        .map_err(|error| format!("w web server could not resolve {:?}: {error}", args.root))?;
    let server_state = ServerState {
        root,
        ifc_artifacts_root: args.ifc_artifacts_root,
        ifc_model_cache: Mutex::new(HashMap::new()),
    };
    let (listener, bound_port) = bind_listener(&args.host, args.port)?;
    let url = format!("http://{}:{}/", args.host, bound_port);

    println!("w web viewer serving {}", server_state.root.display());
    println!(
        "w web query artifacts {}",
        server_state.ifc_artifacts_root.display()
    );
    if bound_port != args.port {
        println!(
            "w web viewer port {} was busy, using {} instead",
            args.port, bound_port
        );
    }
    println!("open {}", url);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(stream, &server_state) {
                    eprintln!("w web server request failed: {error}");
                }
            }
            Err(error) => eprintln!("w web server accept failed: {error}"),
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct Args {
    host: String,
    port: u16,
    root: PathBuf,
    ifc_artifacts_root: PathBuf,
}

struct ServerState {
    root: PathBuf,
    ifc_artifacts_root: PathBuf,
    ifc_model_cache: Mutex<HashMap<String, CachedIfcModel>>,
}

#[derive(Clone)]
struct CachedIfcModel {
    database_stamp: DatabaseStamp,
    model: Arc<VelrIfcModel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DatabaseStamp {
    bytes: u64,
    modified_unix_seconds: u64,
    modified_subsec_nanos: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IfcModelCacheStatus {
    Hit,
    Miss,
    Reloaded,
}

impl IfcModelCacheStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "cache_hit",
            Self::Miss => "cache_miss",
            Self::Reloaded => "cache_reloaded",
        }
    }
}

#[derive(Debug, Clone)]
struct HttpRequest {
    method: String,
    target: String,
    body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CypherApiRequest {
    resource: String,
    cypher: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageApiRequest {
    resource: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct CypherApiResponse {
    resource: String,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    semantic_element_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ApiErrorResponse {
    error: String,
}

#[derive(Debug, Clone)]
struct PackageApiMetrics {
    kind: &'static str,
    cache_status: Option<&'static str>,
    definitions: usize,
    elements: usize,
    instances: usize,
}

#[derive(Debug, Clone)]
struct CypherApiMetrics {
    model_slug: String,
    model_cache_status: &'static str,
    open_ms: u128,
    query_ms: u128,
    extract_ids_ms: u128,
    columns: usize,
    rows: usize,
    semantic_element_ids: usize,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_owned(),
            port: DEFAULT_PORT,
            root: PathBuf::from(DEFAULT_ROOT),
            ifc_artifacts_root: default_ifc_artifacts_root(),
        }
    }
}

impl Args {
    fn parse<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut parsed = Self::default();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--host" => {
                    parsed.host = args
                        .next()
                        .ok_or_else(|| "--host requires a value".to_owned())?;
                }
                "--port" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--port requires a value".to_owned())?;
                    parsed.port = value
                        .parse::<u16>()
                        .map_err(|_| format!("invalid port `{value}`"))?;
                }
                "--root" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--root requires a value".to_owned())?;
                    parsed.root = PathBuf::from(value);
                }
                "--ifc-artifacts-root" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--ifc-artifacts-root requires a value".to_owned())?;
                    parsed.ifc_artifacts_root = PathBuf::from(value);
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => {
                    return Err(format!("unknown argument `{other}`"));
                }
            }
        }

        Ok(parsed)
    }
}

fn print_usage() {
    println!("w web viewer server");
    println!();
    println!("Usage:");
    println!(
        "  cargo run -p cc-w-platform-web --bin cc-w-platform-web-server -- [--host 127.0.0.1] [--port 8001] [--root crates/cc-w-platform-web/artifacts/viewer] [--ifc-artifacts-root artifacts/ifc]"
    );
    println!();
    println!(
        "If the requested port is busy, the server will try the next {} ports.",
        PORT_SEARCH_LIMIT - 1
    );
}

fn bind_listener(host: &str, requested_port: u16) -> Result<(TcpListener, u16), String> {
    let mut last_error = None;

    for port in candidate_ports(requested_port) {
        match TcpListener::bind((host, port)) {
            Ok(listener) => return Ok((listener, port)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                last_error = Some(error);
            }
            Err(error) => {
                return Err(format!(
                    "w web server could not bind {host}:{port}: {error}"
                ));
            }
        }
    }

    let upper_port = requested_port.saturating_add(PORT_SEARCH_LIMIT.saturating_sub(1));
    match last_error {
        Some(error) => Err(format!(
            "w web server could not bind any port in {}-{} on {}: {}",
            requested_port, upper_port, host, error
        )),
        None => Err(format!(
            "w web server did not have any candidate ports to try from {} on {}",
            requested_port, host
        )),
    }
}

fn candidate_ports(start: u16) -> impl Iterator<Item = u16> {
    (0..PORT_SEARCH_LIMIT).map(move |offset| start.saturating_add(offset))
}

fn handle_connection(mut stream: TcpStream, state: &ServerState) -> Result<(), String> {
    let request = read_request(&mut stream)?;
    let request_path = request_path_only(&request.target);

    match request.method.as_str() {
        "GET" | "HEAD" => {
            if request_path == RESOURCES_API_PATH {
                serve_resources_api(&mut stream, request.method == "HEAD", state)
            } else if request_path == CYPHER_API_PATH || request_path == PACKAGE_API_PATH {
                write_json_error(
                    &mut stream,
                    "405 Method Not Allowed",
                    "use POST for package and cypher API routes",
                )
            } else {
                serve_path(
                    &mut stream,
                    request.method == "HEAD",
                    &request.target,
                    &state.root,
                )
            }
        }
        "POST" if request_path == PACKAGE_API_PATH => {
            serve_package_api(&mut stream, &request, state)
        }
        "POST" if request_path == CYPHER_API_PATH => serve_cypher_api(&mut stream, &request, state),
        _ => write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
            false,
        ),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = [0_u8; 1024];
    let mut request = Vec::new();

    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if find_header_end(&request).is_some() {
            break;
        }
        if request.len() >= MAX_REQUEST_HEADER_BYTES {
            return Err("request headers exceeded 16 KiB".to_owned());
        }
    }

    let header_end =
        find_header_end(&request).ok_or_else(|| "request headers were incomplete".to_owned())?;
    let header_text =
        std::str::from_utf8(&request[..header_end]).map_err(|error| error.to_string())?;
    let mut lines = header_text.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "missing HTTP request line".to_owned())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "missing HTTP method".to_owned())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "missing HTTP target".to_owned())?
        .to_string();

    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            if name.trim().eq_ignore_ascii_case("content-length") {
                Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "invalid Content-Length header".to_owned()),
                )
            } else {
                None
            }
        })
        .transpose()?
        .unwrap_or(0);

    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(format!(
            "request body exceeded {} bytes",
            MAX_REQUEST_BODY_BYTES
        ));
    }

    let body_start = header_end + 4;
    while request.len().saturating_sub(body_start) < content_length {
        let read = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("request body ended before Content-Length bytes were read".to_owned());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len().saturating_sub(body_start) > MAX_REQUEST_BODY_BYTES {
            return Err(format!(
                "request body exceeded {} bytes",
                MAX_REQUEST_BODY_BYTES
            ));
        }
    }

    let body = request[body_start..body_start + content_length].to_vec();

    Ok(HttpRequest {
        method,
        target,
        body,
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn request_path_only(target: &str) -> &str {
    target.split('?').next().unwrap_or("/")
}

fn serve_resources_api(
    stream: &mut TcpStream,
    head_only: bool,
    state: &ServerState,
) -> Result<(), String> {
    let payload = WebResourceCatalog {
        resources: available_server_resources(&state.ifc_artifacts_root),
    };
    if head_only {
        return write_response(
            stream,
            "200 OK",
            "application/json; charset=utf-8",
            b"",
            true,
        );
    }
    write_json_response(stream, "200 OK", &payload)
}

fn serve_package_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: PackageApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/package JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();

    let load_started = Instant::now();
    match load_package_response(&api_request.resource, &state.ifc_artifacts_root) {
        Ok((response, metrics)) => {
            let load_ms = load_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] package resource={} kind={} cache_status={} parse_ms={} load_ms={} write_ms={} total_ms={} definitions={} elements={} instances={}",
                api_request.resource,
                metrics.kind,
                metrics.cache_status.unwrap_or("-"),
                parse_ms,
                load_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                metrics.definitions,
                metrics.elements,
                metrics.instances,
            );
            write_result
        }
        Err(error) => {
            let load_ms = load_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            eprintln!(
                "[w web timing] package error resource={} parse_ms={} load_ms={} write_ms={} total_ms={} error={}",
                api_request.resource,
                parse_ms,
                load_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                error,
            );
            write_result
        }
    }
}

fn serve_cypher_api(
    stream: &mut TcpStream,
    request: &HttpRequest,
    state: &ServerState,
) -> Result<(), String> {
    let request_started = Instant::now();
    let parse_started = Instant::now();
    let api_request: CypherApiRequest = serde_json::from_slice(&request.body)
        .map_err(|error| format!("invalid /api/cypher JSON body: {error}"))?;
    let parse_ms = parse_started.elapsed().as_millis();
    let query_preview = summarize_query_for_log(&api_request.cypher);

    let execute_started = Instant::now();
    match execute_cypher_api(&api_request, state) {
        Ok((response, metrics)) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_response(stream, "200 OK", &response);
            let write_ms = write_started.elapsed().as_millis();
            println!(
                "[w web timing] cypher resource={} model={} model_cache={} parse_ms={} open_ms={} query_ms={} ids_ms={} exec_ms={} write_ms={} total_ms={} cols={} rows={} semantic_ids={} query=\"{}\"",
                api_request.resource,
                metrics.model_slug,
                metrics.model_cache_status,
                parse_ms,
                metrics.open_ms,
                metrics.query_ms,
                metrics.extract_ids_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                metrics.columns,
                metrics.rows,
                metrics.semantic_element_ids,
                query_preview,
            );
            write_result
        }
        Err(error) => {
            let execute_ms = execute_started.elapsed().as_millis();
            let write_started = Instant::now();
            let write_result = write_json_error(stream, "400 Bad Request", &error);
            let write_ms = write_started.elapsed().as_millis();
            eprintln!(
                "[w web timing] cypher error resource={} parse_ms={} exec_ms={} write_ms={} total_ms={} query=\"{}\" error={}",
                api_request.resource,
                parse_ms,
                execute_ms,
                write_ms,
                request_started.elapsed().as_millis(),
                query_preview,
                error,
            );
            write_result
        }
    }
}

fn load_package_response(
    resource: &str,
    ifc_artifacts_root: &Path,
) -> Result<(WebPreparedPackageResponse, PackageApiMetrics), String> {
    let (package, metrics) = if let Some(model_slug) = parse_ifc_body_resource(resource) {
        let load = VelrIfcModel::load_body_package_with_cache_status_from_artifacts_root(
            ifc_artifacts_root,
            model_slug,
        )
        .map_err(|error| format!("failed to load IFC package `{resource}`: {error}"))?;
        let summary = load.geometry_summary();
        (
            load.package,
            PackageApiMetrics {
                kind: "ifc",
                cache_status: Some(load.cache_status.as_str()),
                definitions: summary.definitions,
                elements: summary.elements,
                instances: summary.instances,
            },
        )
    } else {
        let package = GeometryBackend::default()
            .build_demo_package_for(resource)
            .map_err(|error| format!("failed to load demo package `{resource}`: {error}"))?;
        let metrics = PackageApiMetrics {
            kind: "demo",
            cache_status: None,
            definitions: package.definitions.len(),
            elements: package.elements.len(),
            instances: package.instances.len(),
        };
        (package, metrics)
    };

    Ok((
        WebPreparedPackageResponse {
            resource: resource.to_string(),
            package: WebPreparedGeometryPackage::from_prepared_package(&package),
        },
        metrics,
    ))
}

fn available_server_resources(ifc_artifacts_root: &Path) -> Vec<String> {
    let mut resources = available_demo_resources()
        .into_iter()
        .map(|resource| resource.to_string())
        .collect::<Vec<_>>();
    if let Ok(mut ifc_resources) = available_ifc_body_resources(ifc_artifacts_root) {
        resources.append(&mut ifc_resources);
    }
    resources.sort();
    resources
}

fn execute_cypher_api(
    request: &CypherApiRequest,
    state: &ServerState,
) -> Result<(CypherApiResponse, CypherApiMetrics), String> {
    if request.cypher.trim().is_empty() {
        return Err("cypher query must not be empty".to_owned());
    }

    let model_slug = parse_ifc_body_resource(&request.resource).ok_or_else(|| {
        format!(
            "cypher queries require an IFC resource like `ifc/building-architecture`; got `{}`",
            request.resource
        )
    })?;
    let open_started = Instant::now();
    let (model, cache_status) = cached_ifc_model(state, model_slug)?;
    let open_ms = open_started.elapsed().as_millis();
    let query_started = Instant::now();
    let query_result = model
        .execute_cypher_rows(&request.cypher)
        .map_err(|error| format!("cypher execution failed for `{model_slug}`: {error}"))?;
    let query_ms = query_started.elapsed().as_millis();
    let extract_started = Instant::now();
    let semantic_element_ids =
        extract_semantic_element_ids(&query_result.columns, &query_result.rows);
    let extract_ids_ms = extract_started.elapsed().as_millis();
    let metrics = CypherApiMetrics {
        model_slug: model_slug.to_owned(),
        model_cache_status: cache_status.as_str(),
        open_ms,
        query_ms,
        extract_ids_ms,
        columns: query_result.columns.len(),
        rows: query_result.rows.len(),
        semantic_element_ids: semantic_element_ids.len(),
    };

    Ok((
        CypherApiResponse {
            resource: request.resource.clone(),
            columns: query_result.columns,
            rows: query_result.rows,
            semantic_element_ids,
        },
        metrics,
    ))
}

fn cached_ifc_model(
    state: &ServerState,
    model_slug: &str,
) -> Result<(Arc<VelrIfcModel>, IfcModelCacheStatus), String> {
    let layout = IfcArtifactLayout::new(&state.ifc_artifacts_root, model_slug);
    let database_stamp = database_stamp(&layout.database)?;

    let cached = {
        let cache = state
            .ifc_model_cache
            .lock()
            .map_err(|_| "IFC model cache lock poisoned".to_owned())?;
        cache.get(model_slug).cloned()
    };
    let had_cached_entry = cached.is_some();

    if let Some(cached) = cached {
        if cached.database_stamp == database_stamp {
            return Ok((cached.model, IfcModelCacheStatus::Hit));
        }
    }

    let model = Arc::new(
        VelrIfcModel::open(layout)
            .map_err(|error| format!("failed to open IFC model `{model_slug}`: {error}"))?,
    );
    let cache_status = if had_cached_entry {
        IfcModelCacheStatus::Reloaded
    } else {
        IfcModelCacheStatus::Miss
    };

    let mut cache = state
        .ifc_model_cache
        .lock()
        .map_err(|_| "IFC model cache lock poisoned".to_owned())?;
    cache.insert(
        model_slug.to_owned(),
        CachedIfcModel {
            database_stamp,
            model: Arc::clone(&model),
        },
    );

    Ok((model, cache_status))
}

fn database_stamp(path: &Path) -> Result<DatabaseStamp, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect database `{}`: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| {
            format!(
                "failed to inspect database mtime `{}`: {error}",
                path.display()
            )
        })?
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            format!(
                "failed to normalize database mtime `{}`: {error}",
                path.display()
            )
        })?;

    Ok(DatabaseStamp {
        bytes: metadata.len(),
        modified_unix_seconds: modified.as_secs(),
        modified_subsec_nanos: modified.subsec_nanos(),
    })
}

fn summarize_query_for_log(query: &str) -> String {
    let collapsed = query.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let preview = chars.by_ref().take(160).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn serve_path(
    stream: &mut TcpStream,
    head_only: bool,
    target: &str,
    root: &Path,
) -> Result<(), String> {
    let relative_path = sanitize_request_path(target)?;
    let file_path = root.join(&relative_path);

    if !file_path.starts_with(root) {
        return write_response(
            stream,
            "403 Forbidden",
            "text/plain; charset=utf-8",
            b"forbidden",
            head_only,
        );
    }

    let bytes = match fs::read(&file_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return write_response(
                stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"not found",
                head_only,
            );
        }
    };

    write_response(
        stream,
        "200 OK",
        content_type_for_path(&file_path),
        &bytes,
        head_only,
    )
}

fn extract_semantic_element_ids(columns: &[String], rows: &[Vec<String>]) -> Vec<String> {
    let explicit_index = find_column_index(columns, &["semanticelementid", "elementid"]);
    let global_id_index = find_column_index(columns, &["globalid"]);
    let product_id_index = find_column_index(columns, &["productid"]);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();

    for row in rows {
        let candidate = explicit_index
            .and_then(|index| row.get(index))
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .or_else(|| {
                global_id_index
                    .and_then(|index| row.get(index))
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            })
            .or_else(|| {
                product_id_index
                    .and_then(|index| row.get(index))
                    .filter(|value| !value.trim().is_empty())
                    .cloned()
            });

        if let Some(id) = candidate {
            if seen.insert(id.clone()) {
                ids.push(id);
            }
        }
    }

    ids
}

fn find_column_index(columns: &[String], candidates: &[&str]) -> Option<usize> {
    columns.iter().position(|column| {
        let normalized = normalize_column_name(column);
        candidates.iter().any(|candidate| normalized == *candidate)
    })
}

fn normalize_column_name(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn sanitize_request_path(target: &str) -> Result<PathBuf, String> {
    let path_only = request_path_only(target);
    let trimmed = path_only.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        PathBuf::from("index.html")
    } else {
        PathBuf::from(trimmed)
    };

    let mut sanitized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("parent directory segments are not allowed".to_owned());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_owned());
            }
        }
    }

    Ok(sanitized)
}

fn content_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| error.to_string())?;
    if !head_only {
        stream.write_all(body).map_err(|error| error.to_string())?;
    }
    stream.flush().map_err(|error| error.to_string())
}

fn write_json_response<T>(stream: &mut TcpStream, status: &str, payload: &T) -> Result<(), String>
where
    T: Serialize,
{
    let body = serde_json::to_vec_pretty(payload)
        .map_err(|error| format!("json encode failed: {error}"))?;
    write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &body,
        false,
    )
}

fn write_json_error(stream: &mut TcpStream, status: &str, error: &str) -> Result<(), String> {
    write_json_response(
        stream,
        status,
        &ApiErrorResponse {
            error: error.to_owned(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        candidate_ports, content_type_for_path, extract_semantic_element_ids, request_path_only,
        sanitize_request_path,
    };
    use std::path::Path;

    #[test]
    fn request_root_maps_to_index() {
        assert_eq!(sanitize_request_path("/").unwrap(), Path::new("index.html"));
    }

    #[test]
    fn request_keeps_nested_assets() {
        assert_eq!(
            sanitize_request_path("/pkg/cc_w_platform_web.js").unwrap(),
            Path::new("pkg/cc_w_platform_web.js")
        );
    }

    #[test]
    fn request_rejects_parent_segments() {
        assert!(sanitize_request_path("/../secret.txt").is_err());
    }

    #[test]
    fn wasm_uses_wasm_content_type() {
        assert_eq!(
            content_type_for_path(Path::new("pkg/cc_w_platform_web_bg.wasm")),
            "application/wasm"
        );
    }

    #[test]
    fn mjs_uses_javascript_content_type() {
        assert_eq!(
            content_type_for_path(Path::new("vendor/xterm.mjs")),
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn candidate_ports_scan_forward_from_requested_port() {
        let ports = candidate_ports(8001).take(4).collect::<Vec<_>>();
        assert_eq!(ports, vec![8001, 8002, 8003, 8004]);
    }

    #[test]
    fn api_path_ignores_query_string() {
        assert_eq!(
            request_path_only("/api/cypher?resource=ifc/building"),
            "/api/cypher"
        );
    }

    #[test]
    fn semantic_id_extraction_prefers_explicit_column() {
        let ids = extract_semantic_element_ids(
            &[String::from("semantic_element_id"), String::from("label")],
            &[
                vec![String::from("A"), String::from("Wall")],
                vec![String::from("A"), String::from("Wall duplicate")],
                vec![String::from("B"), String::from("Door")],
            ],
        );

        assert_eq!(ids, vec!["A", "B"]);
    }

    #[test]
    fn semantic_id_extraction_maps_global_and_product_id_columns() {
        let from_global = extract_semantic_element_ids(
            &[String::from("GlobalId")],
            &[vec![String::from("2xQ$n5SLP5MBLyL442paFx")]],
        );
        assert_eq!(from_global, vec!["2xQ$n5SLP5MBLyL442paFx"]);

        let from_product = extract_semantic_element_ids(
            &[String::from("product_id")],
            &[vec![String::from("42")]],
        );
        assert_eq!(from_product, vec!["42"]);
    }
}

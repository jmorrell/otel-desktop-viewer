//! Private, framed JSON-RPC connection to the bundled collector process.
//! The collector remains the only owner of the DuckDB database.
use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json, value::RawValue};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 128 * 1024 * 1024;
const MAX_REQUEST: usize = 64 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const IMPORT_BATCH: usize = 20 * 1024 * 1024;
type Reply = std::result::Result<Value, String>;

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub executable: PathBuf,
    pub database: PathBuf,
    pub http_port: u16,
    pub grpc_port: u16,
}

impl Default for BackendConfig {
    fn default() -> Self {
        let sibling = std::env::current_exe()
            .unwrap_or_default()
            .with_file_name("otel-desktop-backend");
        let development =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/native/otel-desktop-backend");
        let executable = std::env::var_os("OTEL_DESKTOP_BACKEND")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if sibling.is_file() {
                    sibling
                } else {
                    development
                }
            });
        let database = std::env::var_os("OTEL_DESKTOP_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                directories::ProjectDirs::from("dev", "OpenTelemetry", "Desktop Viewer")
                    .map(|p| p.data_local_dir().join("telemetry.duckdb"))
                    .unwrap_or_else(|| PathBuf::from("telemetry.duckdb"))
            });
        let port = |name: &str, default| {
            std::env::var(name)
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(default)
        };
        Self {
            executable,
            database,
            http_port: port("OTEL_DESKTOP_HTTP_PORT", 4318),
            grpc_port: port("OTEL_DESKTOP_GRPC_PORT", 4317),
        }
    }
}

struct Shared {
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    alive: AtomicBool,
    diagnostics_done: AtomicBool,
    diagnostics: Mutex<VecDeque<String>>,
}

impl Shared {
    fn fail(&self, reason: String) {
        self.alive.store(false, Ordering::Release);
        for (_, tx) in self.pending.lock().unwrap().drain() {
            let _ = tx.send(Err(reason.clone()));
        }
    }
}

/// std::process::Child does not reap on Drop. Guard every partially initialized spawn.
struct SpawnGuard(Option<Child>);
impl Drop for SpawnGuard {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct Backend {
    child: Mutex<Child>,
    input: Mutex<Option<mpsc::SyncSender<Vec<u8>>>>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    pub config: BackendConfig,
}

fn format_rpc_error(error: &Value) -> String {
    let message = error["message"].as_str().unwrap_or("Collector error");
    let mut result = format!("{} (RPC {})", message, error["code"]);
    if let Some(data) = error.get("data").filter(|v| !v.is_null()) {
        let details = data
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| data.to_string());
        if !details.trim().is_empty() && details != message {
            result.push_str(": ");
            result.push_str(&details);
        }
    }
    result
}

impl Backend {
    pub fn spawn(config: BackendConfig) -> Result<Arc<Self>> {
        if config.http_port == 0 || config.grpc_port == 0 {
            bail!(
                "OTLP receiver ports must be in 1..65535; ephemeral ports are not supported by the native handshake"
            );
        }
        if config.http_port == config.grpc_port {
            bail!("OTLP HTTP and gRPC receivers need different ports");
        }
        if let Some(parent) = config.database.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let child = Command::new(&config.executable)
            .arg("--transport=stdio")
            .arg("--open-browser=false")
            .arg("--host=127.0.0.1")
            .arg(format!("--http={}", config.http_port))
            .arg(format!("--grpc={}", config.grpc_port))
            .arg("--db")
            .arg(&config.database)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "Could not start collector at {}. Build the app with native/scripts/package-macos.sh.",
                    config.executable.display()
                )
            })?;
        let mut guard = SpawnGuard(Some(child));
        let child = guard.0.as_mut().unwrap();
        let mut input = child.stdin.take().context("Collector stdin unavailable")?;
        let output = child
            .stdout
            .take()
            .context("Collector stdout unavailable")?;
        let errors = child
            .stderr
            .take()
            .context("Collector stderr unavailable")?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
            diagnostics_done: AtomicBool::new(false),
            diagnostics: Mutex::new(VecDeque::new()),
        });
        // Never block a caller in a pipe write before its request timeout starts.
        // A stopped or overloaded helper has a bounded queue; closing it on Drop
        // requests EOF, and the child shutdown deadline also bounds a stuck writer.
        let (input_tx, input_rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let writer = shared.clone();
        std::thread::Builder::new()
            .name("collector-writer".into())
            .spawn(move || {
                for payload in input_rx {
                    if let Err(error) = write_frame(&mut input, &payload) {
                        writer.fail(format!("Collector write failed: {error:#}"));
                        break;
                    }
                }
            })?;
        let diagnostics = shared.clone();
        std::thread::Builder::new()
            .name("collector-diagnostics".into())
            .spawn(move || {
                for line in BufReader::new(errors)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    eprintln!("{line}");
                    let mut tail = diagnostics.diagnostics.lock().unwrap();
                    if tail.len() >= 20 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
                diagnostics.diagnostics_done.store(true, Ordering::Release);
            })?;
        let reader = shared.clone();
        std::thread::Builder::new()
            .name("collector-rpc".into())
            .spawn(move || {
                let mut output = BufReader::new(output);
                loop {
                    let response = match read_frame(&mut output)
                        .and_then(|bytes| Ok(serde_json::from_slice::<Value>(&bytes)?))
                    {
                        Ok(value) => value,
                        Err(error) => {
                            reader.fail(format!("Collector connection closed: {error:#}"));
                            break;
                        }
                    };
                    if response["jsonrpc"] != "2.0" {
                        reader.fail("Collector sent an invalid JSON-RPC response".into());
                        break;
                    }
                    // Notifications have no id. Only responses consume a pending request.
                    if let Some(id) = response["id"].as_u64() {
                        if let Some(tx) = reader.pending.lock().unwrap().remove(&id) {
                            let result = if response.get("error").is_some() {
                                Err(format_rpc_error(&response["error"]))
                            } else if let Some(result) = response.get("result") {
                                Ok(result.clone())
                            } else {
                                Err("Collector response omitted result and error".into())
                            };
                            let _ = tx.send(result);
                        }
                    }
                }
            })?;
        let backend = Arc::new(Self {
            child: Mutex::new(guard.0.take().unwrap()),
            input: Mutex::new(Some(input_tx)),
            shared,
            next_id: AtomicU64::new(1),
            config,
        });
        let ready = backend
            .call("native.initialize", json!({}))
            .map_err(|error| {
                // Startup error messages are written on stderr just before EOF.
                // Give that reader a bounded opportunity to collect the useful cause.
                let deadline = Instant::now() + Duration::from_millis(250);
                while !backend.shared.diagnostics_done.load(Ordering::Acquire)
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let diagnostics = backend
                    .shared
                    .diagnostics
                    .lock()
                    .unwrap()
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n");
                anyhow!("Collector failed to initialize: {error:#}\n{diagnostics}")
            })?;
        if ready["version"].as_u64() != Some(1) {
            bail!(
                "Unsupported collector protocol version: {}",
                ready["version"]
            );
        }
        Ok(backend)
    }

    #[cfg(test)]
    pub fn child_pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::Acquire)
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.request(method, params, REQUEST_TIMEOUT)
    }

    #[cfg(test)]
    pub fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.request(method, params, timeout)
    }

    fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if !self.shared.alive.load(Ordering::Acquire) {
            bail!("Collector is no longer running");
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.shared.pending.lock().unwrap();
            // The reader marks dead before draining this same lock; no request can
            // be inserted behind a disconnect and wait for a reply that cannot arrive.
            if !self.shared.alive.load(Ordering::Acquire) {
                bail!("Collector is no longer running");
            }
            pending.insert(id, tx);
        }
        if let Err(error) =
            self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
        {
            self.shared.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        match rx.recv_timeout(timeout) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(anyhow!(error)),
            Err(error) => {
                self.shared.pending.lock().unwrap().remove(&id);
                let _ = self
                    .send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}));
                Err(anyhow!("{method} did not finish: {error}"))
            }
        }
    }

    fn send(&self, message: Value) -> Result<()> {
        let payload = serde_json::to_vec(&message)?;
        if payload.len() > MAX_REQUEST {
            bail!("RPC request exceeds 64 MiB");
        }
        let guard = self.input.lock().unwrap();
        let input = guard.as_ref().context("Collector connection has closed")?;
        input.try_send(payload).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                anyhow!("Collector request queue is full; try again after it responds")
            }
            mpsc::TrySendError::Disconnected(_) => anyhow!("Collector connection has closed"),
        })
    }

    pub fn import_file(&self, path: &Path) -> Result<ImportReport> {
        self.import_file_cancellable(path, &AtomicBool::new(false))
    }

    pub fn import_file_cancellable(
        &self,
        path: &Path,
        cancelled: &AtomicBool,
    ) -> Result<ImportReport> {
        let file =
            std::fs::File::open(path).with_context(|| format!("Opening {}", path.display()))?;
        let mut report = ImportReport::default();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        // A stream accepts formatted JSON, JSONL and adjacent JSON objects. RawValue
        // retains received numbers and negative zero rather than round-tripping them.
        let stream = serde_json::Deserializer::from_reader(BufReader::new(file))
            .into_iter::<BTreeMap<String, Box<RawValue>>>();
        for document in stream {
            if cancelled.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            let document = document.context("Invalid OTLP JSON")?;
            for (key, raw) in document {
                let signal = match key.as_str() {
                    "resourceSpans" => "traces",
                    "resourceLogs" => "logs",
                    "resourceMetrics" => "metrics",
                    "resourceProfiles" => {
                        report.issues.push("Profiles are not supported".into());
                        continue;
                    }
                    _ => {
                        report
                            .issues
                            .push(format!("Unrecognized OTLP wrapper: {key}"));
                        continue;
                    }
                };
                let resources: Vec<Box<RawValue>> = match serde_json::from_str(raw.get()) {
                    Ok(resources) => resources,
                    Err(error) => {
                        report
                            .issues
                            .push(format!("{key} must contain resource objects: {error}"));
                        continue;
                    }
                };
                for resource in resources {
                    let mut pieces = Vec::new();
                    split_otlp(
                        resource.get(),
                        signal,
                        0,
                        IMPORT_BATCH - key.len() - 8,
                        &mut pieces,
                    )?;
                    for piece in pieces {
                        if cancelled.load(Ordering::Relaxed) {
                            report.cancelled = true;
                            return Ok(report);
                        }
                        let body = format!("{{\"{key}\":[{piece}]}}");
                        if body.len() > IMPORT_BATCH {
                            report.issues.push(format!(
                                "An indivisible {signal} record exceeds the 20 MiB import limit"
                            ));
                            continue;
                        }
                        let response = client
                            .post(format!(
                                "http://127.0.0.1:{}/v1/{signal}",
                                self.config.http_port
                            ))
                            .header("Content-Type", "application/json")
                            .body(body)
                            .send()?;
                        let status = response.status();
                        let body = response.text()?;
                        if !status.is_success() {
                            report
                                .issues
                                .push(format!("{signal}: HTTP {status}: {body}"));
                            continue;
                        }
                        report.requests += 1;
                        if let Ok(value) = serde_json::from_str::<Value>(&body) {
                            if let Some(partial) = value.get("partialSuccess") {
                                if partial.as_object().is_some_and(|v| !v.is_empty()) {
                                    report.issues.push(format!("{signal}: {partial}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        if report.requests == 0 && report.issues.is_empty() && !report.cancelled {
            report.issues.push("No OTLP records were found".into());
        }
        Ok(report)
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // EOF asks the helper to drain the collector before it exits. Bound the
        // wait even if a crashed or incompatible helper fails to honor it.
        self.input.get_mut().unwrap().take();
        let child = self.child.get_mut().unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub requests: usize,
    pub issues: Vec<String>,
    pub cancelled: bool,
}
impl std::fmt::Display for ImportReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} import batches accepted{}",
            self.requests,
            if self.cancelled { " · cancelled" } else { "" }
        )?;
        if !self.issues.is_empty() {
            write!(f, " · {}", self.issues.join("; "))?;
        }
        Ok(())
    }
}

pub(crate) fn read_frame(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut length = None;
    let mut header_size = 0;
    loop {
        let mut line = String::new();
        let read = reader.take(8193).read_line(&mut line)?;
        if read == 0 {
            bail!("end of stream");
        }
        header_size += read;
        if header_size > 8192 {
            bail!("RPC header exceeds 8 KiB");
        }
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').context("Malformed RPC header")?;
        if name.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                bail!("Duplicate Content-Length");
            }
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .context("Invalid Content-Length")?,
            );
        }
    }
    let length = length.context("Missing Content-Length")?;
    if length == 0 || length > MAX_FRAME {
        bail!("RPC frame size {length} is outside 1..={MAX_FRAME}");
    }
    let mut payload = vec![0; length];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

fn write_frame(writer: &mut impl Write, payload: &[u8]) -> Result<()> {
    if payload.len() > MAX_FRAME {
        bail!("RPC message exceeds 128 MiB");
    }
    write!(writer, "Content-Length: {}\r\n\r\n", payload.len())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

// Split known collection boundaries only. Opaque attributes/body bytes are
// copied exactly; all resource, scope and instrument metadata accompanies each
// resulting batch. This also handles one very large resource or metric stream.
fn split_otlp(
    raw: &str,
    signal: &str,
    depth: usize,
    limit: usize,
    out: &mut Vec<String>,
) -> Result<()> {
    if raw.len() <= limit {
        out.push(raw.to_owned());
        return Ok(());
    }
    let object: BTreeMap<String, Box<RawValue>> = serde_json::from_str(raw)?;
    let collection = match depth {
        0 => Some(match signal {
            "traces" => "scopeSpans",
            "logs" => "scopeLogs",
            _ => "scopeMetrics",
        }),
        1 => Some(match signal {
            "traces" => "spans",
            "logs" => "logRecords",
            _ => "metrics",
        }),
        2 if signal == "metrics" => [
            "gauge",
            "sum",
            "histogram",
            "exponentialHistogram",
            "summary",
        ]
        .into_iter()
        .find(|k| object.contains_key(*k)),
        3 if signal == "metrics" => Some("dataPoints"),
        _ => None,
    };
    let Some(key) = collection.filter(|key| object.contains_key(*key)) else {
        out.push(raw.to_owned());
        return Ok(());
    };
    let replace = |replacement: &str| {
        let mut result = String::from("{");
        for (index, (name, value)) in object.iter().enumerate() {
            if index != 0 {
                result.push(',');
            }
            result.push_str(&serde_json::to_string(name).unwrap());
            result.push(':');
            result.push_str(if name == key {
                replacement
            } else {
                value.get()
            });
        }
        result.push('}');
        result
    };
    let overhead = replace("").len() + 2;
    let remaining = limit.saturating_sub(overhead);
    if depth == 2 {
        let mut chunks = Vec::new();
        split_otlp(object[key].get(), signal, depth + 1, remaining, &mut chunks)?;
        out.extend(chunks.iter().map(|chunk| replace(chunk)));
    } else {
        let items: Vec<Box<RawValue>> = serde_json::from_str(object[key].get())?;
        let mut batch = String::new();
        for item in items {
            let mut chunks = Vec::new();
            split_otlp(item.get(), signal, depth + 1, remaining, &mut chunks)?;
            for chunk in chunks {
                if !batch.is_empty() && batch.len() + chunk.len() + 1 > remaining {
                    out.push(replace(&format!("[{batch}]")));
                    batch.clear();
                }
                if !batch.is_empty() {
                    batch.push(',');
                }
                batch.push_str(&chunk);
            }
        }
        if !batch.is_empty() {
            out.push(replace(&format!("[{batch}]")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rpc_errors_include_structured_data() {
        assert_eq!(
            super::format_rpc_error(
                &serde_json::json!({"code":-32602,"message":"Invalid params","data":{"field":"sort"}})
            ),
            "Invalid params (RPC -32602): {\"field\":\"sort\"}"
        );
        assert_eq!(
            super::format_rpc_error(
                &serde_json::json!({"code":-32602,"message":"Invalid params","data":null})
            ),
            "Invalid params (RPC -32602)"
        );
    }

    #[test]
    fn framing_keeps_utf8_lengths_and_multiple_messages() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, "{\"x\":\"京都\"}".as_bytes()).unwrap();
        write_frame(&mut bytes, b"null").unwrap();
        let mut reader = std::io::Cursor::new(bytes);
        assert_eq!(
            read_frame(&mut reader).unwrap(),
            "{\"x\":\"京都\"}".as_bytes()
        );
        assert_eq!(read_frame(&mut reader).unwrap(), b"null");
    }
    #[test]
    fn invalid_frames_are_rejected_before_allocation() {
        for invalid in [
            "Content-Length: 3\r\nContent-Length: 3\r\n\r\nabc",
            "Content-Length: 134217729\r\n\r\n",
            "Content-Length: -1\r\n\r\n",
            "X: 1\r\n\r\n",
        ] {
            assert!(read_frame(&mut std::io::Cursor::new(invalid)).is_err());
        }
    }
    #[test]
    fn import_splitting_keeps_exact_numbers_and_metadata() {
        let raw = r#"{"resource":{"attributes":[{"key":"n","value":{"intValue":9223372036854775807}}]},"scopeSpans":[{"scope":{"name":"test"},"spans":[{"name":"a","x":-0.0},{"name":"b","x":1e309}]}]}"#;
        let mut out = Vec::new();
        split_otlp(raw, "traces", 0, 170, &mut out).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|part| part.len() <= 170));
        assert!(
            out.iter()
                .all(|part| part.contains("9223372036854775807")
                    && part.contains(r#""name":"test""#))
        );
        assert!(out[0].contains("-0.0"));
        assert!(out[1].contains("1e309"));
    }
}

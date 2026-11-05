//! Exercises the real child Collector, OTLP importer, stdio RPC and DuckDB.
#[path = "../src/backend.rs"]
mod backend;
#[path = "../src/model.rs"]
mod model;

use backend::{Backend, BackendConfig};
use serde_json::{Value, json};
use std::{
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

static INTEGRATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn eventually<F: FnMut() -> bool>(mut f: F) {
    let until = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < until, "collector did not flush fixtures");
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn root() -> PathBuf {
    std::env::var_os("OTEL_NATIVE_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
}
fn find_metric(rows: &Value, name: &str) -> String {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .expect("metric exists")["metricRef"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn real_import_paging_exact_details_and_scoped_exports() {
    let _guard = INTEGRATION_LOCK.lock().unwrap();
    let helper = std::env::var_os("OTEL_DESKTOP_BACKEND")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("dist/native/otel-desktop-backend"));
    assert!(
        helper.is_file(),
        "build dist/native/otel-desktop-backend or set OTEL_DESKTOP_BACKEND before running the integration test: {}",
        helper.display()
    );
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp = std::env::temp_dir().join(format!(
        "otel-native-integration-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let http_port = free_port();
    let mut grpc_port = free_port();
    while grpc_port == http_port {
        grpc_port = free_port();
    }
    let backend = Backend::spawn(BackendConfig {
        executable: helper,
        database: temp.join("data.duckdb"),
        http_port,
        grpc_port,
    })
    .expect("start collector");
    for signal in ["traces", "logs", "metrics"] {
        let report = backend
            .import_file(&root().join(format!("testdata/otlp/checkout/{signal}.json")))
            .expect("import checkout fixture");
        assert!(report.issues.is_empty(), "{report:?}");
    }
    let exact_fixture = temp.join("exact-metrics.json");
    std::fs::write(&exact_fixture,br#"{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"precision-service"}}]},"scopeMetrics":[{"scope":{"name":"precision-test"},"metrics":[{"name":"precision.integer","gauge":{"dataPoints":[{"timeUnixNano":"1791360001400000000","asInt":"9223372036854775807","attributes":[{"key":"dimension","value":{"stringValue":"first"}}]},{"timeUnixNano":"1791360001500000000","asInt":"-9223372036854775808","attributes":[{"key":"dimension","value":{"stringValue":"second"}}]}]}},{"name":"precision.zero","gauge":{"dataPoints":[{"timeUnixNano":"1791360001600000000","asDouble":-0.0}]}}]}]}]}"#).unwrap();
    let report = backend
        .import_file(&exact_fixture)
        .expect("import exact values");
    assert!(report.issues.is_empty(), "{report:?}");
    eventually(|| {
        let s = backend.call("getStats", json!([])).unwrap();
        s["logs"]["logCount"].as_u64().unwrap_or(0) >= 3
            && s["metrics"]["dataPointCount"].as_u64().unwrap_or(0) >= 10
    });

    let base = json!({"startTime":null,"endTime":null,"query":null,"limit":null,"sort":{"field":"timestamp","direction":"desc"},"pageSize":1,"offset":0,"columns":[{"source":"resource","kind":"attribute","key":"service.name"},{"source":"log","kind":"field","key":"traceId"}]});
    let first = backend.call("searchLogPage", base.clone()).unwrap();
    assert_eq!(first["total"], 3);
    let log = &first["logs"][0];
    let log_ref = log["logRef"].as_str().unwrap();
    assert_eq!(log["columnValues"][0]["values"][0]["value"], "api-gateway");
    assert_eq!(
        log["columnValues"][1]["values"][0]["value"],
        "4bf92f3577b34da6a3ce929d0e0e4736"
    );
    let definitions = backend
        .call("getLogAttributeDefinitions", json!([]))
        .unwrap();
    let definitions = definitions.as_array().unwrap();
    assert!(definitions.iter().any(|d| d["name"] == "region"
        && d["attributeScope"] == "resource"
        && d["type"] == "string"));
    let attribute_matches = backend
        .call("searchAttributeMatches", json!({"term":"west"}))
        .unwrap();
    assert!(
        attribute_matches
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["name"] == "region" && d["attributeScope"] == "resource")
    );
    let completions = backend
        .call(
            "getFieldValueCompletions",
            json!({"signal":"traces","field":"name","term":"payment","limit":30}),
        )
        .unwrap();
    assert!(
        completions
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "payment/charge")
    );
    let shorthand =
        model::parse_search_request_with_fields("region = \"west\"", "logs", definitions)
            .unwrap()
            .0;
    let region_results = backend.call("searchLogPage", json!({"startTime":null,"endTime":null,"query":shorthand,"limit":null,"pageSize":25,"offset":0})).unwrap();
    assert!(region_results["total"].as_u64().unwrap() > 0);
    let severity_query = model::parse_search("severityNumber >= 17", "logs").unwrap();
    let severity_results = backend.call("searchLogPage", json!({"startTime":null,"endTime":null,"query":severity_query,"limit":null,"sort":{"field":"timestamp","direction":"desc"},"pageSize":25,"offset":0})).unwrap();
    assert_eq!(severity_results["total"], 2);
    let text_query = model::parse_search("Payment declined", "logs").unwrap();
    let text_results = backend.call("searchLogPage", json!({"startTime":null,"endTime":null,"query":text_query,"limit":null,"pageSize":25,"offset":0})).unwrap();
    assert_eq!(text_results["total"], 1);
    let duration_query = model::parse_search("duration >= 1s", "traces").unwrap();
    let duration_results = backend
        .call(
            "searchTraceSummaries",
            json!({"startTime":null,"endTime":null,"query":duration_query,"limit":null}),
        )
        .unwrap();
    assert!(!duration_results.as_array().unwrap().is_empty());
    let mut second_params = base.clone();
    second_params["offset"] = json!(1);
    let second = backend.call("searchLogPage", second_params).unwrap();
    assert_ne!(first["logs"][0]["logRef"], second["logs"][0]["logRef"]);
    let navigation=backend.call("getLogErrorNavigation",json!({"startTime":null,"endTime":null,"query":null,"limit":null,"sort":{"field":"timestamp","direction":"desc"},"selectedLogRef":log_ref})).unwrap();
    assert_eq!(navigation["count"], 2);
    assert_eq!(navigation["next"]["offset"], 1);
    assert_eq!(navigation["next"]["logRef"], second["logs"][0]["logRef"]);
    let log_detail = backend.call("getLog", json!({"logRef":log_ref})).unwrap();
    let trace_id = log_detail["traceID"].as_str().unwrap();
    assert_eq!(trace_id, "4bf92f3577b34da6a3ce929d0e0e4736");
    let trace = backend
        .call("getTraceView", json!({"traceID":trace_id,"query":null}))
        .unwrap();
    assert_eq!(trace["spans"].as_array().unwrap().len(), 6);
    let log_export = backend
        .call(
            "native.export",
            json!({"signal":"logs","reference":log_ref}),
        )
        .unwrap();
    let exported_log: Value =
        serde_json::from_str(log_export["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        exported_log["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let summaries = backend
        .call(
            "searchMetricSummaries",
            json!({"startTime":null,"endTime":null,"query":null,"limit":null}),
        )
        .unwrap();
    let metric_id = find_metric(&summaries, "precision.integer");
    let metric = backend
        .call("getMetric", json!({"metricRef":metric_id}))
        .unwrap();
    assert_eq!(metric["series"].as_array().unwrap().len(), 2);
    let series_id = metric["series"][0]["seriesRef"].as_str().unwrap();
    let series = backend
        .call(
            "getMetricSeries",
            json!({"metricRef":metric_id,"seriesRef":series_id,"startTime":null,"endTime":null}),
        )
        .unwrap();
    let point = &series["datapoints"][0];
    let integer = point["intValue"].as_str().unwrap();
    assert!(integer == "9223372036854775807" || integer == "-9223372036854775808");
    let point_id = point["datapointRef"].as_str().unwrap();
    let metric_export = backend
        .call(
            "native.export",
            json!({"signal":"metrics","reference":metric_id}),
        )
        .unwrap();
    let definition_export = backend
        .call(
            "native.export",
            json!({"signal":"metrics","reference":metric_id,"includeDatapoints":false}),
        )
        .unwrap();
    let definition_otlp: Value =
        serde_json::from_str(definition_export["content"].as_str().unwrap()).unwrap();
    let definition_points = &definition_otlp["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]
        ["gauge"]["dataPoints"];
    assert!(definition_points.is_null() || definition_points.as_array().is_some_and(Vec::is_empty));
    let series_export = backend
        .call(
            "native.export",
            json!({"signal":"metrics","reference":metric_id,"scope":{"seriesRef":series_id}}),
        )
        .unwrap();
    let point_export = backend
        .call(
            "native.export",
            json!({"signal":"metrics","reference":metric_id,"scope":{"datapointRef":point_id}}),
        )
        .unwrap();
    assert!(
        metric_export["content"]
            .as_str()
            .unwrap()
            .contains("9223372036854775807")
    );
    assert!(point_export["content"].as_str().unwrap().contains(integer));
    let series_otlp: Value =
        serde_json::from_str(series_export["content"].as_str().unwrap()).unwrap();
    let point_otlp: Value =
        serde_json::from_str(point_export["content"].as_str().unwrap()).unwrap();
    for export in [&series_otlp, &point_otlp] {
        assert_eq!(
            export["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["gauge"]["dataPoints"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    let histogram_id = find_metric(&summaries, "http.server.request.duration");
    let histogram = backend
        .call("getMetric", json!({"metricRef":histogram_id}))
        .unwrap();
    assert_eq!(histogram["metricType"], "Histogram");
    assert_eq!(histogram["aggregationTemporalityCode"], 1);
    let histogram_series = histogram["series"][0]["seriesRef"].as_str().unwrap();
    let histogram_exact = backend.call("getMetricSeries", json!({"metricRef":histogram_id,"seriesRef":histogram_series,"startTime":null,"endTime":null})).unwrap();
    let histogram_point = &histogram_exact["datapoints"][0];
    assert_eq!(histogram_point["count"], "10");
    let aggregate_params = json!({"metricRef":histogram_id,"startTime":null,"endTime":null,"targetBuckets":1,"seriesRefs":[histogram_series],"quantiles":[],"viewBuckets":0});
    let aggregate = backend
        .call("getMetricAggregateView", aggregate_params.clone())
        .unwrap();
    let mut contribution_params = aggregate_params;
    contribution_params["datapointRef"] = histogram_point["datapointRef"].clone();
    let contribution = backend
        .call("getMetricAggregateView", contribution_params)
        .unwrap();
    assert_eq!(
        contribution["aggregate"], aggregate["aggregate"],
        "selected contribution must not replace whole aggregate"
    );
    assert_eq!(contribution["contribution"].as_array().unwrap().len(), 1);
    assert_eq!(contribution["contribution"][0]["count"], 10);
    assert_eq!(histogram_point["exemplars"][0]["traceID"], trace_id);
    let exemplar_trace = backend
        .call(
            "getTraceView",
            json!({"traceID":histogram_point["exemplars"][0]["traceID"],"query":null}),
        )
        .unwrap();
    assert_eq!(exemplar_trace["spans"].as_array().unwrap().len(), 6);
    let histogram_export = backend.call("native.export", json!({"signal":"metrics","reference":histogram_id,"scope":{"datapointRef":histogram_point["datapointRef"]}})).unwrap();
    let histogram_otlp: Value =
        serde_json::from_str(histogram_export["content"].as_str().unwrap()).unwrap();
    assert_eq!(
        histogram_otlp["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"]["dataPoints"]
            [0]["exemplars"][0]["traceId"],
        trace_id
    );

    let zero_id = find_metric(&summaries, "precision.zero");
    let zero = backend
        .call("getMetric", json!({"metricRef":zero_id}))
        .unwrap();
    let zero_series = zero["series"][0]["seriesRef"].as_str().unwrap();
    let zero_point = backend
        .call(
            "getMetricSeries",
            json!({"metricRef":zero_id,"seriesRef":zero_series,"startTime":null,"endTime":null}),
        )
        .unwrap();
    assert_eq!(
        zero_point["datapoints"][0]["doubleValue"],
        "0x8000000000000000"
    );
    let zero_export = backend
        .call(
            "native.export",
            json!({"signal":"metrics","reference":zero_id}),
        )
        .unwrap();
    let zero_otlp: Value = serde_json::from_str(zero_export["content"].as_str().unwrap()).unwrap();
    let exported_zero =
        zero_otlp["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["gauge"]["dataPoints"][0]
            ["asDouble"]
            .as_f64()
            .unwrap();
    assert!(exported_zero == 0.0 && exported_zero.is_sign_negative());
    let identities_fixture = temp.join("identity-logs.json");
    std::fs::write(&identities_fixture, br#"{"resourceLogs":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"same"}},{"key":"service.namespace","value":{"stringValue":"a"}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":"1791360001700000000","body":{"stringValue":"identity-a"}}]}]},{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"same"}},{"key":"service.namespace","value":{"stringValue":"b"}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":"1791360001800000000","body":{"stringValue":"identity-b"}}]}]},{"resource":{"attributes":[{"key":"service.name","value":{"intValue":"7"}}]},"scopeLogs":[{"logRecords":[{"timeUnixNano":"1791360001900000000","body":{"stringValue":"identity-int"}}]}]}]}"#).unwrap();
    assert!(
        backend
            .import_file(&identities_fixture)
            .unwrap()
            .issues
            .is_empty()
    );
    eventually(|| {
        backend.call("getStats", json!([])).unwrap()["logs"]["logCount"]
            .as_u64()
            .unwrap_or(0)
            >= 6
    });
    let services = backend
        .call(
            "discoverTelemetry",
            json!({"options":{"kind":"services","limit":500}}),
        )
        .unwrap();
    let entries = services["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| &row[0])
        .collect::<Vec<_>>();
    for (name, namespace, kind, expected) in [
        ("same", "a", "string", "identity-a"),
        ("same", "b", "string", "identity-b"),
        ("7", "", "int64", "identity-int"),
    ] {
        let service = entries
            .iter()
            .find(|s| {
                s["serviceName"] == name
                    && s["serviceNamespace"] == namespace
                    && s["serviceNameKind"] == kind
            })
            .expect("typed service identity");
        let identity = service["serviceIdentity"].as_str().unwrap();
        let query = json!({"id":"test-service","type":"condition","query":{"field":{"searchScope":"field","name":"serviceIdentity","type":"string"},"fieldOperator":"=","value":identity}});
        let page = backend.call("searchLogPage", json!({"startTime":null,"endTime":null,"query":query,"limit":null,"pageSize":25,"offset":0})).unwrap();
        assert_eq!(page["total"], 1);
        assert_eq!(page["logs"][0]["bodyPreview"], expected);
    }
    drop(backend);
    let _ = std::fs::remove_dir_all(temp);
}

#[test]
fn helper_shutdown_reaps_and_crash_resolves_concurrent_calls() {
    let _guard = INTEGRATION_LOCK.lock().unwrap();
    use std::sync::{Arc, Barrier, mpsc};
    let helper = std::env::var_os("OTEL_DESKTOP_BACKEND")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("dist/native/otel-desktop-backend"));
    assert!(helper.is_file(), "build helper first: {}", helper.display());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp = std::env::temp_dir().join(format!(
        "otel-native-lifecycle-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let occupied_port = occupied.local_addr().unwrap().port();
    let failure_grpc = free_port();
    let start = Instant::now();
    let failure = Backend::spawn(BackendConfig {
        executable: helper.clone(),
        database: temp.join("startup-error.duckdb"),
        http_port: occupied_port,
        grpc_port: failure_grpc,
    });
    let failure_text = match failure {
        Ok(_) => panic!("occupied HTTP port must fail startup"),
        Err(error) => format!("{error:#}"),
    };
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "startup failure must not wait for the RPC timeout"
    );
    assert!(
        failure_text.contains("address already in use"),
        "startup diagnostics must include the bind error: {failure_text}"
    );
    assert!(
        TcpListener::bind(("127.0.0.1", failure_grpc)).is_ok(),
        "failed collector must release gRPC port"
    );
    drop(occupied);
    let grpc_port = free_port();
    let mut http_port = free_port();
    while http_port == grpc_port {
        http_port = free_port();
    }
    let config = BackendConfig {
        executable: helper,
        database: temp.join("data.duckdb"),
        http_port,
        grpc_port,
    };
    let backend = Backend::spawn(config.clone()).expect("start collector");
    let pid = backend.child_pid();
    drop(backend);
    assert!(
        TcpListener::bind(("127.0.0.1", http_port)).is_ok(),
        "HTTP receiver must close after Drop"
    );
    assert!(
        TcpListener::bind(("127.0.0.1", grpc_port)).is_ok(),
        "gRPC receiver must close after Drop"
    );
    assert!(
        !std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success(),
        "dropped helper must be reaped"
    );
    // The same database and receiver ports can be reused after graceful shutdown.
    let backend = Backend::spawn(config).expect("reopen after graceful shutdown");
    let pid = backend.child_pid();
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-STOP", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    // Always resume even if the previous synchronous-write implementation
    // blocks: then elapsed-time assertion catches the regression without hanging.
    let resume = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        assert!(
            std::process::Command::new("/bin/kill")
                .args(["-CONT", &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
    });
    let started = Instant::now();
    let stopped_result = backend.call_with_timeout(
        "getStats",
        json!({"padding":"x".repeat(256 * 1024)}),
        Duration::from_millis(250),
    );
    let elapsed = started.elapsed();
    resume.join().unwrap();
    assert!(stopped_result.is_err(), "stopped helper must time out");
    assert!(
        elapsed < Duration::from_secs(1),
        "pipe backpressure bypassed deadline: {elapsed:?}"
    );
    assert!(
        backend.call("getStats", json!([])).is_ok(),
        "helper must recover after resume"
    );
    let start = Arc::new(Barrier::new(5));
    let (tx, rx) = mpsc::channel();
    for _ in 0..4 {
        let b = backend.clone();
        let gate = start.clone();
        let tx = tx.clone();
        std::thread::spawn(move || {
            gate.wait();
            let result = b.call(
                "query",
                json!({"sql":"SELECT sum(sqrt(i)) FROM range(10000000000) AS r(i)","limit":1}),
            );
            let _ = tx.send(result);
        });
    }
    drop(tx);
    start.wait();
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "queries must still be in flight before crash"
    );
    assert!(
        std::process::Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    for _ in 0..4 {
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("disconnect must promptly resolve every pending call");
        assert!(
            result.is_err(),
            "crashed helper cannot return a successful query"
        );
    }
    drop(backend);
    assert!(
        !std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success(),
        "crashed helper must be reaped"
    );
    let _ = std::fs::remove_dir_all(temp);
}

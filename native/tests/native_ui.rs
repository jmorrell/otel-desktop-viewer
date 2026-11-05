//! In-process production-UI checks with real stdio Collector/DuckDB responses.
//! Screenshots come from GPUI's offscreen Metal renderer, not an OS desktop capture.
#[path = "../src/app.rs"]
mod app;
#[path = "../src/app_actions.rs"]
mod app_actions;
#[path = "../src/backend.rs"]
mod backend;
#[path = "../src/detail_stack.rs"]
mod detail_stack;
#[path = "../src/home.rs"]
mod home;
#[path = "../src/logs.rs"]
mod logs;
#[path = "../src/metric_chart.rs"]
mod metric_chart;
#[path = "../src/metric_details.rs"]
mod metric_details;
#[path = "../src/metrics.rs"]
mod metrics;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/search.rs"]
mod search;
#[path = "../src/shell.rs"]
mod shell;
#[path = "../src/theme.rs"]
mod theme;
#[path = "../src/traces.rs"]
mod traces;
#[path = "../src/widgets.rs"]
mod widgets;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Focusable, HeadlessAppContext, MouseButton, ScrollDelta,
    WindowBounds, WindowOptions, assets, init, open_window, platform, point, px, size,
    test::{TestEventExt, TestWindowExt},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
struct Recording {
    directory: PathBuf,
    index: usize,
    last: Instant,
    started: Instant,
}
thread_local! {static RECORDING:std::cell::RefCell<Option<Recording>>=const{std::cell::RefCell::new(None)};}
fn record_frame(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    let target = RECORDING.with(|state| {
        let mut state = state.borrow_mut();
        let r = state.as_mut()?;
        if r.last.elapsed() < Duration::from_millis(100) {
            return None;
        }
        r.last = Instant::now();
        Some((
            r.directory.clone(),
            r.index,
            if r.index == 0 {
                0
            } else {
                (r.started.elapsed().as_millis() / 100) as usize
            },
        ))
    });
    if let Some((directory, index, target_index)) = target {
        let image = cx.capture_screenshot(window).unwrap();
        if image.width() != 2880 || image.height() != 1800 {
            return;
        }
        if index > 0 {
            for duplicate in index..target_index {
                std::fs::copy(
                    directory.join(format!("frame-{:05}.png", index - 1)),
                    directory.join(format!("frame-{duplicate:05}.png")),
                )
                .unwrap();
            }
        }
        let actual_index = target_index.max(index);
        image
            .save(directory.join(format!("frame-{actual_index:05}.png")))
            .unwrap();
        RECORDING.with(|state| {
            if let Some(r) = state.borrow_mut().as_mut() {
                r.index = target_index.max(index) + 1;
            }
        });
    }
}
fn pump(cx: &mut HeadlessAppContext, window: AnyWindowHandle, millis: u64) {
    let deadline = Instant::now() + Duration::from_millis(millis);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(12));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        record_frame(cx, window);
    }
}
fn wait_model(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    view: &gpui_kit::Entity<app::Viewer>,
    description: &str,
    predicate: impl Fn(&app::Viewer) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if cx.update(|cx| predicate(view.read(cx))) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}: {}",
            cx.update(|cx| view.read(cx).status.clone())
        );
        pump(cx, window, 80);
    }
}
fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let artifacts = std::env::var_os("OTEL_NATIVE_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("otel-native-headless-{}", std::process::id()))
        });
    let artifacts = artifacts.join(format!(
        "run-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        std::process::id()
    ));
    std::fs::create_dir_all(&artifacts).unwrap();
    eprintln!("Headless Metal artifacts: {}", artifacts.display());
    let config = backend::BackendConfig {
        executable: root.join("dist/native/otel-desktop-backend"),
        database: artifacts.join("ui.duckdb"),
        http_port: free_port(),
        grpc_port: free_port(),
    };
    let backend = backend::Backend::spawn(config).expect("real Collector startup");
    for signal in ["traces", "logs", "metrics"] {
        let report = backend
            .import_file(&root.join(format!("testdata/otlp/checkout/{signal}.json")))
            .expect("fixture imports through OTLP receiver");
        assert!(report.issues.is_empty(), "{report}");
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let stats = backend.call("getStats", json!([])).unwrap();
        if model::ns(&stats["traces"]["spanCount"]) >= 6
            && model::ns(&stats["logs"]["logCount"]) >= 3
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "collector must flush real fixtures: {stats}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    let trace_summaries = backend
        .call(
            "searchTraceSummaries",
            json!({"startTime":null,"endTime":null,"query":null,"limit":1000}),
        )
        .unwrap();
    let trace_id = model::field(&trace_summaries[0], "traceID");
    assert!(!trace_id.is_empty());
    let trace = backend
        .call("getTraceView", json!({"traceID":trace_id}))
        .unwrap();
    assert_eq!(trace["spans"].as_array().unwrap().len(), 6);
    let mut cx = HeadlessAppContext::with_platform(
        platform::current_platform(true).text_system(),
        Arc::new(assets::Assets),
        platform::current_headless_renderer,
    );
    cx.update(|cx| {
        init(cx);
        app::register_actions(cx);
        traces::register_actions(cx);
        cx.text_system()
            .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/AtkinsonHyperlegibleNext.ttf"
            ))])
            .unwrap();
    });
    let (handle, view) = cx
        .update(|cx| {
            open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(1440.), px(900.)),
                    })),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut view = app::Viewer::with_backend(window, cx, backend.clone());
                        view.page = app::Page::Traces;
                        view.data
                            .insert("searchTraceSummaries".into(), trace_summaries);
                        view.selected_trace = Some(trace_id.clone());
                        view.trace_view = trace.clone();
                        view
                    })
                },
            )
        })
        .unwrap();
    let frames = artifacts.join("frames");
    std::fs::create_dir_all(&frames).unwrap();
    RECORDING.with(|state| {
        *state.borrow_mut() = Some(Recording {
            directory: frames,
            index: 0,
            last: Instant::now() - Duration::from_secs(1),
            started: Instant::now(),
        })
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("search").bounds().size.width > px(100.));
        assert!(window.find("trace-list").visible());
        assert!(window.find("span-details").visible());
        assert!(window.find("collapse-spans").visible());
        window.click("collapse-spans", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(view.read(cx).collapsed.len(), 6));
    cx.update_window(handle, |_, window, cx| {
        window.click("expand-spans", cx);
        window.click("next-span-error", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 180);
    cx.update(|cx| {
        assert!(
            !view
                .read(cx)
                .selected_span
                .as_deref()
                .unwrap_or_default()
                .is_empty()
        );
        assert!(view.read(cx).collapsed.is_empty());
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("copy-span", cx);
        let copied = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        let parsed: Value = serde_json::from_str(&copied).unwrap();
        assert_eq!(parsed["traceID"], trace_id);
        assert!(parsed["startTime"].is_string());
        assert!(parsed["resource"].is_object());
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let start = window
            .find("detail-resize-traces:summary")
            .bounds()
            .center();
        window.pointer_down(start, MouseButton::Left, cx);
        window.pointer_move(start + point(px(0.), px(36.)), Some(MouseButton::Left), cx);
        window.pointer_up(start + point(px(0.), px(36.)), MouseButton::Left, cx);
    })
    .unwrap();
    cx.update(|cx| assert!(view.read(cx).prefs.detail_layout["traces"][0].height > 300.));
    let day = cx
        .capture_screenshot(handle)
        .expect("real offscreen Metal screenshot");
    assert!(day.width() >= 1440 && day.height() >= 900);
    day.save(artifacts.join("trace-day-metal.png")).unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("theme", cx);
    })
    .unwrap();
    let night = cx.capture_screenshot(handle).unwrap();
    assert_ne!(day.as_raw(), night.as_raw(), "theme changes actual pixels");
    night.save(artifacts.join("trace-night-metal.png")).unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("appearance", cx);
        assert!(window.find(("season", 0usize)).visible());
        window.click(("season", 0usize), cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(view.read(cx).prefs.season, 0));
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("appearance-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("appearance", cx);
        view.read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    })
    .unwrap();
    pump(&mut cx, handle, 150);
    cx.update_window(handle, |_, window, cx| {
        window.input("traceI", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 150);
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(view.read(cx).search.read(cx).value(), "link.traceID ");
        window.press("cmd-a", cx);
        window.press("backspace", cx);
        window.input("traceI", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 150);
    cx.update_window(handle, |_, window, cx| {
        window.click(("search-suggestion", 0usize), cx);
        assert_eq!(view.read(cx).search.read(cx).value(), "traceID ");
        window.press("cmd-a", cx);
        window.press("backspace", cx);
        window.input("statusCode = Error", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 220);
    cx.update(|cx| assert_eq!(view.read(cx).search.read(cx).value(), "statusCode = Error"));
    cx.update_window(handle, |_, window, cx| {
        window.click("list", cx);
        assert!(window.try_find("trace-list").is_none());
        window.click("list", cx);
        assert!(window.find("trace-list").visible());
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        let start = window.find("resize-details").bounds().center();
        window.pointer_down(start, MouseButton::Left, cx);
        window.pointer_move(start + point(px(-60.), px(0.)), Some(MouseButton::Left), cx);
        window.pointer_up(start + point(px(-60.), px(0.)), MouseButton::Left, cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert!(
            view.read(cx).prefs.details_width > 360.,
            "native drag changes shared saved width"
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("search", cx);
        window.press("cmd-a", cx);
        window.press("backspace", cx);
        window.press("enter", cx);
        window.click("Home", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 700);
    cx.update(|cx| {
        assert!(view.read(cx).data.get("home.services").is_some());
        assert!(
            view.read(cx).status == "Collector ready · OTLP HTTP / gRPC",
            "{}",
            view.read(cx).status
        );
    });
    // Derive the actual painted segment from received service/connection identities.
    wait_model(&mut cx, handle, &view, "service map edges", |v| {
        v.data.iter().any(|(k, v)| {
            k.starts_with("home.connections.") && !model::discovery_rows(v).is_empty()
        })
    });
    let edge_midpoint = cx.update(|cx| {
        let v = view.read(cx);
        let mut services = model::discovery_rows(&v.data["home.services"]);
        services.sort_by_key(|s| {
            (
                model::field(s, "serviceName"),
                model::field(s, "serviceNamespace"),
            )
        });
        let edge = v
            .data
            .iter()
            .filter(|(k, _)| k.starts_with("home.connections."))
            .flat_map(|(_, v)| model::discovery_rows(v))
            .next()
            .unwrap();
        let position = |identity: &Value| {
            let i = services
                .iter()
                .position(|s| s["serviceIdentity"] == *identity)
                .unwrap();
            let angle = i as f32 / services.len() as f32 * std::f32::consts::TAU
                - std::f32::consts::FRAC_PI_2;
            (0.5 + angle.cos() * 0.36, 0.5 + angle.sin() * 0.36)
        };
        let a = position(&edge["source"]["serviceIdentity"]);
        let b = position(&edge["target"]["serviceIdentity"]);
        ((a.0 + b.0) / 2., (a.1 + b.1) / 2.)
    });
    cx.update_window(handle, |_, window, cx| {
        let b = window.find("service-map-canvas").bounds();
        let hit = b.origin
            + point(
                b.size.width * edge_midpoint.0,
                b.size.height * edge_midpoint.1,
            );
        window.pointer_down(hit, MouseButton::Left, cx);
        window.pointer_up(hit, MouseButton::Left, cx);
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "painted service edge", |v| {
        v.data
            .get("home.selected-edge")
            .is_some_and(|v| v.is_object())
            && v.data.get("home.connection-spans").is_some()
    });
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("home-map-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click(("home-service", 0usize), cx);
    })
    .unwrap();
    pump(&mut cx, handle, 500);
    cx.update(|cx| assert!(view.read(cx).selected_service.is_some()));
    wait_model(&mut cx, handle, &view, "Home attribute keys", |v| {
        v.data.get("home.keys").is_some()
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("expand-all-attribute-values", cx)
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "expanded attribute values", |v| {
        v.data
            .keys()
            .any(|k| k.starts_with("home.expanded-values."))
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(
            window
                .try_find(("attribute-values-preview", 0usize))
                .is_some()
        );
        window.click("collapse-all-attribute-values", cx);
        assert!(
            window
                .try_find(("attribute-values-preview", 0usize))
                .is_none()
        );
    })
    .unwrap();
    cx.update(|cx| {
        let rows = model::discovery_rows(&view.read(cx).data["home.keys"]);
        assert!(!rows.is_empty());
        for key in rows {
            assert!(model::ns(&key["count"]) > 0);
            assert!(model::ns(&key["denominator"]) >= model::ns(&key["count"]));
            assert!(key["relativeFrequency"].is_number());
        }
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find(("attribute-key", 0usize)).visible());
        window.click("home-keys-sort", cx);
        window.click("home-keys-sort", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(view.read(cx).data["home.keys-sort.traces"], "count");
        assert_eq!(view.read(cx).data["home.keys-direction.traces"], "desc");
    });
    pump(&mut cx, handle, 200);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("home-attributes-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| window.click("Logs", cx))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update(|cx| {
        assert!(
            view.read(cx).data["logs.page"]["logs"]
                .as_array()
                .is_some_and(|v| v.len() >= 3)
        );
        assert!(
            view.read(cx).status == "Collector ready · OTLP HTTP / gRPC",
            "{}",
            view.read(cx).status
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("cell-0-body", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 200);
    cx.update(|cx| assert!(view.read(cx).selected_log.is_some()));
    cx.update_window(handle, |_, window, cx| {
        for section in ["summary", "attributes", "resource", "scope", "body", "otlp"] {
            assert!(
                window
                    .try_find(format!("detail-section-logs:{section}"))
                    .is_some(),
                "missing log section {section}"
            );
        }
        window.scroll(
            "detail-collapse-logs:summary",
            ScrollDelta::Pixels(point(px(0.), px(-5000.))),
            cx,
        );
        assert!(window.find("raw-toggle-log").visible());
        window.click("raw-toggle-log", cx);
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "lazy exact log OTLP", |v| {
        v.data
            .get("ui.raw.open")
            .and_then(Value::as_str)
            .and_then(|key| v.data.get(key))
            .is_some_and(|v| v["content"].is_string())
    });
    cx.update(|cx| {
        let v = view.read(cx);
        let key = v.data["ui.raw.open"].as_str().unwrap();
        assert!(key.starts_with(&format!(
            "ui.raw.logs.{}.",
            v.selected_log.as_deref().unwrap()
        )));
        let otlp: Value = serde_json::from_str(v.data[key]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            otlp["resourceLogs"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| r["scopeLogs"].as_array().unwrap())
                .flat_map(|s| s["logRecords"].as_array().unwrap())
                .count(),
            1
        );
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("raw-toggle-log", cx);
        window.scroll(
            "raw-toggle-log",
            ScrollDelta::Pixels(point(px(0.), px(5000.))),
            cx,
        );
    })
    .unwrap();
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("logs-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| window.click("Metrics", cx))
        .unwrap();
    pump(&mut cx, handle, 350);
    wait_model(&mut cx, handle, &view, "metric summaries", |view| {
        view.data
            .get("searchMetricSummaries")
            .is_some_and(|v| v.as_array().is_some_and(|v| v.len() >= 3))
    });
    let histogram_index = cx.update(|cx| {
        view.read(cx).data["searchMetricSummaries"]
            .as_array()
            .unwrap()
            .iter()
            .position(|m| model::field(m, "metricType").contains("Histogram"))
            .unwrap()
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(("metric", histogram_index), cx)
    })
    .unwrap();
    pump(&mut cx, handle, 600);
    cx.update_window(handle, |_, window, _| {
        for section in [
            "summary",
            "metadata",
            "resource",
            "scope",
            "series",
            "datapoints",
            "otlp",
        ] {
            assert!(
                window
                    .try_find(format!("detail-section-metrics:{section}"))
                    .is_some(),
                "missing metric section {section}"
            );
        }
    })
    .unwrap();
    wait_model(
        &mut cx,
        handle,
        &view,
        "histogram metric response",
        |view| {
            view.data
                .get("metrics.view")
                .is_some_and(|v| model::field(v, "metricType").contains("Histogram"))
                && !view.metrics.visible.is_empty()
                && view.data.contains_key("metrics.histogram-summary")
        },
    );
    cx.update(|cx| {
        assert!(
            model::field(&view.read(cx).data["metrics.view"], "metricType").contains("Histogram")
        );
        assert!(!view.read(cx).metrics.visible.is_empty());
        assert!(
            view.read(cx).status == "Collector ready · OTLP HTTP / gRPC",
            "{}",
            view.read(cx).status
        );
    });
    let quantile_hidden =
        cx.update(|cx| view.read(cx).metrics.prefs.hidden_quantiles.contains("0.5"));
    cx.update_window(handle, |_, window, cx| window.click("quantile-0.5", cx))
        .unwrap();
    cx.update(|cx| {
        assert_ne!(
            view.read(cx).metrics.prefs.hidden_quantiles.contains("0.5"),
            quantile_hidden
        )
    });
    cx.update_window(handle, |_, window, cx| window.click("quantile-0.5", cx))
        .unwrap();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).metrics.prefs.hidden_quantiles.contains("0.5"),
            quantile_hidden
        )
    });
    cx.update_window(handle, |_, window, cx| {
        window.click(("series-select", 0usize), cx)
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "received histogram series", |v| {
        v.data
            .get("metrics.series")
            .is_some_and(|s| s["datapoints"].as_array().is_some_and(|p| !p.is_empty()))
    });
    cx.update_window(handle, |_, window, cx| {
        window.scroll(
            "detail-collapse-metrics:summary",
            ScrollDelta::Pixels(point(px(0.), px(-5000.))),
            cx,
        );
        assert!(window.find(("metric-detail-point", 0usize)).visible());
        window.click(("metric-detail-point", 0usize), cx);
    })
    .unwrap();
    wait_model(
        &mut cx,
        handle,
        &view,
        "received datapoint contribution",
        |v| {
            v.data
                .get("metrics.contribution")
                .is_some_and(|r| r["contribution"].as_array().is_some_and(|p| !p.is_empty()))
        },
    );
    cx.update(|cx| {
        let v = view.read(cx);
        assert_eq!(
            v.data["metrics.contribution.request"]["datapointRef"].as_str(),
            v.selected_point.as_deref()
        );
        assert_eq!(
            v.data["metrics.contribution.request"]["seriesRefs"]
                .as_array()
                .unwrap()
                .len(),
            v.metrics.visible.len()
        );
    });
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-datapoint-contribution-metal.png"))
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.scroll(
            "raw-toggle-metric",
            ScrollDelta::Pixels(point(px(0.), px(5000.))),
            cx,
        )
    })
    .unwrap();
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-heatmap-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("Histogram", cx);
        window.click("metric-chart", cx);
        window.press("right", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 300);
    cx.update(|cx| {
        assert!(!view.read(cx).metrics.prefs.heatmap);
        assert!(view.read(cx).metrics.hover.is_some());
    });
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-histogram-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| window.click("toggle-series", cx))
        .unwrap();
    pump(&mut cx, handle, 250);
    cx.update(|cx| assert!(view.read(cx).metrics.visible.is_empty()));
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-no-series-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| window.click("toggle-series", cx))
        .unwrap();
    pump(&mut cx, handle, 250);
    cx.update(|cx| assert!(!view.read(cx).metrics.visible.is_empty()));
    for number in 1..=12 {
        let report = backend
            .import_file(&root.join(format!("testdata/otlp/demo/metrics-{number:02}.json")))
            .expect("real richer metrics import");
        assert!(report.issues.is_empty(), "{report}");
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let stats = backend.call("getStats", json!([])).unwrap();
        if model::ns(&stats["metrics"]["dataPointCount"]) >= 9908 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "demo metrics must flush: {stats}"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
    cx.update_window(handle, |_, window, cx| window.click("refresh", cx))
        .unwrap();
    wait_model(&mut cx, handle, &view, "richer metric summaries", |view| {
        view.data
            .get("searchMetricSummaries")
            .and_then(Value::as_array)
            .is_some_and(|v| {
                v.iter().any(|m| {
                    model::field(m, "metricType").contains("Histogram")
                        && model::ns(&m["dataPointCount"]) > 10
                })
            })
    });
    let (index, reference) = cx.update(|cx| {
        let rows = view.read(cx).data["searchMetricSummaries"]
            .as_array()
            .unwrap();
        let index = rows
            .iter()
            .position(|m| {
                model::field(m, "metricType").contains("Histogram")
                    && model::ns(&m["dataPointCount"]) > 10
            })
            .unwrap();
        (index, model::field(&rows[index], "metricRef"))
    });
    cx.update_window(handle, |_, window, cx| window.click(("metric", index), cx))
        .unwrap();
    wait_model(&mut cx, handle, &view, "multi-bucket histogram", |view| {
        view.data
            .get("metrics.view")
            .is_some_and(|v| v["metricRef"] == reference)
            && view
                .data
                .get("metrics.histogram-summary")
                .is_some_and(|v| v["aggregate"].as_array().is_some_and(|v| !v.is_empty()))
    });
    cx.update_window(handle, |_, window, cx| window.click("Heatmap", cx))
        .unwrap();
    pump(&mut cx, handle, 500);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-demo-heatmap-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("metric-chart", cx);
        window.press("end", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 250);
    cx.update(|cx| assert!(view.read(cx).metrics.cursor > 0));
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-demo-selection-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| window.click("Histogram", cx))
        .unwrap();
    pump(&mut cx, handle, 200);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-demo-histogram-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        window.press("cmd-a", cx);
        window.press("backspace", cx);
        window.input("name = \"messaging.queue.depth\"", cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "scalar name filter", |v| {
        v.data["searchMetricSummaries"]
            .as_array()
            .is_some_and(|rows| {
                rows.len() == 2
                    && rows
                        .iter()
                        .all(|m| model::field(m, "name") == "messaging.queue.depth")
            })
    });
    let (index, reference) = cx.update(|cx| {
        let rows = view.read(cx).data["searchMetricSummaries"]
            .as_array()
            .unwrap();
        let index = rows
            .iter()
            .position(|m| model::ns(&m["dataPointCount"]) > 10)
            .unwrap();
        (index, model::field(&rows[index], "metricRef"))
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find(("metric", index)).visible());
        window.click(("metric", index), cx)
    })
    .unwrap();
    wait_model(&mut cx, handle, &view, "multi-point scalar", |view| {
        view.data
            .get("metrics.view")
            .is_some_and(|v| v["metricRef"] == reference)
    });
    pump(&mut cx, handle, 300);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-demo-scalar-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    let invalid = artifacts.join("invalid-import.json");
    std::fs::write(&invalid, b"{invalid JSON").unwrap();
    cx.update(|cx| {
        view.update(cx, |view, cx| {
            view.import_paths(
                vec![root.join("testdata/otlp/checkout/logs.json"), invalid],
                cx,
            )
        })
    });
    wait_model(&mut cx, handle, &view, "serial import results", |view| {
        !view.import_active
            && view
                .data
                .get("import.report")
                .is_some_and(|v| v.as_array().is_some_and(|v| v.len() == 2))
    });
    cx.update_window(handle, |_, window, cx| {
        assert!(window.find("import-results").visible());
        window.click("copy-import-results", cx);
        let report: Value =
            serde_json::from_str(&cx.read_from_clipboard().unwrap().text().unwrap()).unwrap();
        assert_eq!(report.as_array().unwrap().len(), 2);
        assert!(
            report
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["error"].is_string())
        );
    })
    .unwrap();
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("import-results-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        window.click("close-import-results", cx)
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.resize(size(px(900.), px(760.)));
        window.bounds_changed(cx);
        assert_eq!(window.viewport_size(), size(px(900.), px(760.)));
        window.render_frame(cx);
    })
    .unwrap();
    pump(&mut cx, handle, 150);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("metric-900px-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update_window(handle, |_, window, cx| {
        view.read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        window.press("cmd-a", cx);
        window.press("backspace", cx);
        window.click("Traces", cx);
    })
    .unwrap();
    pump(&mut cx, handle, 300);
    cx.capture_screenshot(handle)
        .unwrap()
        .save(artifacts.join("trace-900px-metal.png"))
        .unwrap();
    pump(&mut cx, handle, 350);
    cx.update(|cx| {
        assert!(
            view.read(cx).status == "Import finished · 2 files · 3 batches",
            "{}",
            view.read(cx).status
        )
    });
    RECORDING.with(|state|{if let Some(r)=state.borrow().as_ref(){std::fs::write(artifacts.join("recording.json"),serde_json::to_vec_pretty(&json!({"renderer":"GPUI Metal offscreen","source":"production Viewer with real stdio Collector/DuckDB backend, checkout and demo OTLP imports, and native input events","fps":10,"frames":r.index,"width":2880,"height":1800,"desktopCapture":false})).unwrap()).unwrap();}});
    println!(
        "PASS: production GPUI with real Collector/DuckDB, trace collapse/error navigation, exact clipboard, search keyboard/pointer, pane/section resize, themes, painted Home edge, attribute expansion, lazy raw OTLP, log/metric sections, quantile toggle, datapoint contribution, rich charts, import reports, and 900px layout. Offscreen Metal artifacts: {}",
        artifacts.display()
    );
}

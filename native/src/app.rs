use crate::{
    backend::{Backend, BackendConfig},
    theme::{color, palette},
};
use gpui_kit::{
    component::input::{InputEvent, InputState},
    *,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
    time::Duration,
};
actions!(
    viewer,
    [
        Refresh,
        FocusSearch,
        GoHome,
        GoTraces,
        GoLogs,
        GoMetrics,
        DismissOverlay
    ]
);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Home,
    Traces,
    Logs,
    Metrics,
}
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub dark: bool,
    pub timezone: String,
    pub season: usize,
    pub ink: usize,
    pub list_open: bool,
    pub details_open: bool,
    pub list_width: f32,
    pub details_width: f32,
    pub trace_label_width: f32,
    pub detail_layout: HashMap<String, Vec<crate::detail_stack::SectionPreference>>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            dark: false,
            timezone: "Local".into(),
            season: 2,
            ink: usize::MAX,
            list_open: true,
            details_open: true,
            list_width: 290.,
            details_width: 360.,
            trace_label_width: 260.,
            detail_layout: HashMap::new(),
        }
    }
}
#[derive(Default)]
pub(crate) struct TimeRange {
    pub start: Option<String>,
    pub end: Option<String>,
    pub relative_minutes: Option<i64>,
    pub label: String,
}
pub(crate) struct Viewer {
    pub backend: Option<Arc<Backend>>,
    pub page: Page,
    pub prefs: Preferences,
    pub persist_preferences: bool,
    pub toolbar_height: f32,
    pub search: Entity<InputState>,
    pub search_state: crate::search::SearchState,
    pub home_filter: Entity<InputState>,
    pub time_start: Entity<InputState>,
    pub time_end: Entity<InputState>,
    pub timezone_input: Entity<InputState>,
    pub time_open: bool,
    pub time_range: TimeRange,
    pub status: String,
    pub loading: bool,
    pub new_data: bool,
    pub appearance_open: bool,
    pub appearance_focus: FocusHandle,
    pub appearance_trigger_focus: FocusHandle,
    pub resizing: Option<(bool, f32, f32)>,
    pub trace_resizing: Option<(f32, f32)>,
    pub detail_resizing: Option<(String, String, f32, f32)>,
    pub trace_label_render_width: f32,
    pub import_cancel: Arc<std::sync::atomic::AtomicBool>,
    pub import_active: bool,
    pub import_report_open: bool,
    pub data: HashMap<String, Value>,
    pub selected_trace: Option<String>,
    pub selected_span: Option<String>,
    pub selected_log: Option<String>,
    pub selected_metric: Option<String>,
    pub selected_series: Option<String>,
    pub selected_point: Option<String>,
    pub selected_service: Option<(String, String)>,
    pub trace_view: Value,
    pub logs_offset: usize,
    pub metrics_offset: usize,
    pub page_size: usize,
    pub metrics: crate::metrics::MetricsState,
    pub collapsed: std::collections::HashSet<String>,
    pub trace_sort: usize,
    pub trace_matches_only: bool,
    pub trace_reveal: bool,
    pub trace_scroll: UniformListScrollHandle,
    pub trace_focus: FocusHandle,
    pub trace_card_focus: HashMap<String, FocusHandle>,
    pub trace_parent_ids: std::collections::HashSet<String>,
    pub generations: HashMap<String, u64>,
    pub _subscriptions: Vec<Subscription>,
}
impl Viewer {
    fn initial(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search telemetry"));
        let timezone_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Local, UTC, or IANA timezone"));
        let timezone_subscription =
            cx.subscribe_in(&timezone_input, window, |_, _, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            });
        let time_start =
            cx.new(|cx| InputState::new(window, cx).placeholder("Start · ISO 8601 or nanoseconds"));
        let time_end =
            cx.new(|cx| InputState::new(window, cx).placeholder("End · ISO 8601 or nanoseconds"));
        let home_filter = cx
            .new(|cx| InputState::new(window, cx).placeholder("Filter services, keys, and values"));
        let home_subscription = cx.subscribe_in(&home_filter, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.refresh_home(cx);
            }
        });
        let subscription =
            cx.subscribe_in(&search, window, |this, _, event, window, cx| match event {
                InputEvent::Change => this.search_changed(cx),
                InputEvent::Focus => {
                    this.refresh_search_definitions(cx);
                    this.update_search_completions(cx);
                }
                InputEvent::Blur => {}
                InputEvent::PressEnter { .. } => this.submit_search(window, cx),
            });
        let mut this = Self {
            backend: None,
            page: Page::Home,
            prefs: Default::default(),
            persist_preferences: true,
            toolbar_height: 48.,
            search,
            search_state: Default::default(),
            home_filter,
            time_start,
            time_end,
            timezone_input,
            time_open: false,
            time_range: Default::default(),
            status: "Starting collector…".into(),
            loading: true,
            new_data: false,
            appearance_open: false,
            appearance_focus: cx.focus_handle(),
            appearance_trigger_focus: cx.focus_handle(),
            resizing: None,
            trace_resizing: None,
            detail_resizing: None,
            trace_label_render_width: 260.,
            import_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            import_active: false,
            import_report_open: false,
            data: HashMap::new(),
            selected_trace: None,
            selected_span: None,
            selected_log: None,
            selected_metric: None,
            selected_series: None,
            selected_point: None,
            selected_service: None,
            trace_view: Value::Null,
            logs_offset: 0,
            metrics_offset: 0,
            page_size: 50,
            metrics: Default::default(),
            collapsed: Default::default(),
            trace_sort: 0,
            trace_matches_only: false,
            trace_reveal: false,
            trace_scroll: UniformListScrollHandle::new(),
            trace_focus: cx.focus_handle(),
            trace_card_focus: HashMap::new(),
            trace_parent_ids: Default::default(),
            generations: HashMap::new(),
            _subscriptions: vec![subscription, home_subscription, timezone_subscription],
        };
        this.prefs.dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        if let Ok(s) = std::fs::read_to_string(Self::prefs_path()) {
            if let Ok(p) = serde_json::from_str(&s) {
                this.prefs = p;
            }
        }
        if let Err(error) = crate::model::set_timezone(&this.prefs.timezone) {
            this.status = format!("Timezone: {error}");
        }
        this.timezone_input.update(cx, |input, cx| {
            input.set_value(this.prefs.timezone.clone(), window, cx)
        });
        crate::theme::selection(this.prefs.season, this.prefs.ink);
        crate::theme::apply(this.prefs.dark, cx);
        this
    }
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::initial(window, cx);
        this.start_backend(cx);
        cx.spawn(async move |view, cx| {
            loop {
                smol::Timer::after(Duration::from_secs(3)).await;
                if view
                    .update(cx, |this, cx| this.rpc("getStats", json!([]), cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    pub fn start_backend(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        let previous = self.backend.take();
        for generation in self.generations.values_mut() {
            *generation += 1;
        }
        self.status = "Starting collector…".into();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            drop(previous);
            let _ = tx.send(Backend::spawn(BackendConfig::default()));
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        let _ = view.update(cx, |this, cx| {
                            this.loading = false;
                            match result {
                                Ok(backend) => {
                                    this.backend = Some(backend);
                                    this.status = "Collector ready · OTLP HTTP / gRPC".into();
                                    this.refresh_search_definitions(cx);
                                    this.refresh(cx);
                                }
                                Err(e) => this.status = format!("Collector failed: {e:#}"),
                            };
                            cx.notify();
                        });
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        smol::Timer::after(Duration::from_millis(30)).await;
                    }
                    Err(_) => break,
                }
            }
        })
        .detach();
        cx.notify();
    }
    pub fn with_backend(
        window: &mut Window,
        cx: &mut Context<Self>,
        backend: Arc<Backend>,
    ) -> Self {
        let mut this = Self::initial(window, cx);
        this.backend = Some(backend);
        this.persist_preferences = false;
        let _ = crate::model::set_timezone("Local");
        this.prefs = Default::default();
        this.timezone_input
            .update(cx, |input, cx| input.set_value("Local", window, cx));
        this.loading = false;
        this.status = "Collector ready · OTLP HTTP / gRPC".into();
        crate::theme::selection(this.prefs.season, this.prefs.ink);
        crate::theme::apply(this.prefs.dark, cx);
        this
    }
    fn prefs_path() -> std::path::PathBuf {
        directories::ProjectDirs::from("dev", "OpenTelemetry", "Desktop Viewer")
            .map(|p| p.config_dir().join("preferences.json"))
            .unwrap_or_else(|| "native-preferences.json".into())
    }
    pub fn save_preferences(&self) {
        if !self.persist_preferences {
            return;
        }
        let path = Self::prefs_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&self.prefs) {
            let _ = std::fs::write(path, text);
        }
    }
    pub fn rpc(&mut self, method: &str, params: Value, cx: &mut Context<Self>) {
        self.rpc_key(method, method, params, cx)
    }
    pub fn rpc_key(&mut self, key: &str, method: &str, params: Value, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let key = key.to_owned();
        let method = method.to_owned();
        let generation = self.generations.entry(key.clone()).or_default();
        *generation += 1;
        let generation = *generation;
        let method_for_error = method.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(backend.call(&method, params));
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        let _ = view.update(cx, |this, cx| {
                            if this.generations.get(&key) != Some(&generation) {
                                return;
                            }
                            match result {
                                Ok(value) => {
                                    if key == "getTraceView" {
                                        this.trace_view = value.clone();
                                        if let Some(spans) = value["spans"].as_array() {
                                            let retained =
                                                this.selected_span.as_deref().and_then(|id| {
                                                    spans.iter().find(|n| {
                                                        n["spanData"]["spanID"].as_str() == Some(id)
                                                    })
                                                });
                                            let node = retained
                                                .or_else(|| {
                                                    spans.iter().find(|n| {
                                                        n["matched"].as_bool().unwrap_or(false)
                                                    })
                                                })
                                                .or_else(|| spans.first());
                                            if let Some(id) =
                                                node.and_then(|n| n["spanData"]["spanID"].as_str())
                                            {
                                                this.select_span(id.to_owned(), cx);
                                            }
                                        }
                                    }
                                    if key == "getStats" {
                                        if let Some(previous) = this.data.get(&key) {
                                            if stats_received_new(previous, &value) {
                                                this.new_data = true;
                                            }
                                        }
                                    }
                                    let mutation_finished = key.ends_with(".deleted");
                                    let trace_deleted = key == "trace.delete";
                                    let home_services = key == "home.services"
                                        || key.starts_with("home.services.more.");
                                    this.data.insert(key, value);
                                    if home_services {
                                        this.refresh_home_connections(cx);
                                    }
                                    if mutation_finished {
                                        this.refresh(cx);
                                    }
                                    if trace_deleted {
                                        this.selected_trace = None;
                                        this.selected_span = None;
                                        this.trace_view = Value::Null;
                                        this.data.remove("ui.trace.delete");
                                        this.refresh(cx);
                                    }
                                }
                                Err(error) => {
                                    eprintln!(
                                        "native RPC {method_for_error} ({key}) failed: {error:#}"
                                    );
                                    this.status = format!("{error:#}");
                                }
                            };
                            cx.notify();
                        });
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        smol::Timer::after(Duration::from_millis(20)).await;
                    }
                    Err(_) => break,
                }
            }
        })
        .detach();
    }
    pub fn current_params(&self, _cx: &App) -> Value {
        let signal = match self.page {
            Page::Home => "all",
            Page::Traces => "traces",
            Page::Logs => "logs",
            Page::Metrics => "metrics",
        };
        let applied = self.data.get(&format!("search.applied.{signal}"));
        json!({"query":applied.map(|a|a["query"].clone()).unwrap_or(Value::Null),"startTime":self.time_range.start,"endTime":self.time_range.end,"limit":applied.map(|a|a["limit"].clone()).unwrap_or(Value::Null)})
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.page != Page::Home {
            let signal = match self.page {
                Page::Traces => "traces",
                Page::Logs => "logs",
                Page::Metrics => "metrics",
                Page::Home => "all",
            };
            let input = self.search.read(cx).value().to_string();
            match crate::model::parse_search_request_with_fields(
                &input,
                signal,
                &self.current_search_definitions(),
            ) {
                Ok((query, limit)) => {
                    let previous = self.data.get(&format!("search.applied.{signal}"));
                    if previous.and_then(|p| p["input"].as_str()) != Some(input.as_str()) {
                        self.logs_offset = 0;
                        self.metrics_offset = 0;
                    }
                    self.data.insert(
                        format!("search.applied.{signal}"),
                        json!({"input":input,"query":query,"limit":limit}),
                    );
                }
                Err(error) => {
                    self.status = format!("Search: {error}");
                    cx.notify();
                    return;
                }
            }
        }
        if let Some(minutes) = self.time_range.relative_minutes {
            self.time_range.start = Some(
                (chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128
                    - minutes as i128 * 60_000_000_000)
                    .to_string(),
            );
        }
        self.new_data = false;
        self.rpc("getStats", json!([]), cx);
        let mut params = self.current_params(cx);
        if self.page == Page::Logs {
            params["pageSize"] = json!(self.page_size);
            params["offset"] = json!(self.logs_offset);
        }
        if self.page == Page::Traces {
            params["sort"] = json!({"field":(["startTime","duration","rootSpanName","serviceName","spanCount","errorCount"][(self.trace_sort/2)%6]),"direction":if self.trace_sort%2==0 {"desc"}else{"asc"}});
        }
        match self.page {
            Page::Home => {
                self.refresh_home(cx);
            }
            Page::Traces => {
                let query = params["query"].clone();
                self.rpc("searchTraceSummaries", params, cx);
                if let Some(id) = self.selected_trace.clone() {
                    self.rpc("getTraceView", json!({"traceID":id,"query":query}), cx);
                    self.rpc("getTraceLogSummaries", json!({"traceID":id}), cx);
                }
            }
            Page::Logs => self.refresh_logs(cx),
            Page::Metrics => self.refresh_metrics(cx),
        }
        cx.notify();
    }
    pub fn select_trace(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_search_completions(cx);
        self.data.remove("ui.trace.event");
        self.data.remove("ui.trace.delete");
        self.selected_trace = Some(id.clone());
        self.page = Page::Traces;
        self.trace_view = Value::Null;
        self.selected_span = None;
        let query = self.current_params(cx)["query"].clone();
        self.rpc("getTraceView", json!({"traceID":id,"query":query}), cx);
        self.rpc("getTraceLogSummaries", json!({"traceID":id}), cx);
        cx.notify();
    }
    pub fn select_log(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_search_completions(cx);
        self.selected_log = Some(id.clone());
        self.page = Page::Logs;
        self.load_log(id, cx);
        cx.notify();
    }
    pub fn select_metric(&mut self, id: String, cx: &mut Context<Self>) {
        self.close_search_completions(cx);
        self.page = Page::Metrics;
        self.load_metric(id, cx);
        cx.notify();
    }
}
impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(self.prefs.dark);
        self.toolbar_height = if f32::from(window.viewport_size().width) < 1100. {
            88.
        } else {
            48.
        };
        let content = match self.page {
            Page::Home => self.render_home(window, cx),
            Page::Traces => self.render_traces(window, cx),
            Page::Logs => self.render_logs(window, cx),
            Page::Metrics => self.render_metrics(window, cx),
        };
        div()
            .id("viewer")
            .test_support()
            .relative()
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.import_paths(paths.0.iter().cloned().collect(), cx)
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some((signal, id, origin, height)) = this.detail_resizing.clone() {
                    if event.dragging() {
                        if let Some(layout) = this.prefs.detail_layout.get_mut(&signal) {
                            if let Some(index) = layout.iter().position(|s| s.id == id) {
                                let desired = (height + f32::from(event.position.y) - origin)
                                    .clamp(80., 1200.);
                                let mut delta = desired - layout[index].height;
                                if let Some(next) =
                                    ((index + 1)..layout.len()).find(|i| !layout[*i].collapsed)
                                {
                                    delta = delta.min(layout[next].height - 80.);
                                    layout[next].height =
                                        (layout[next].height - delta).clamp(80., 1200.);
                                }
                                layout[index].height =
                                    (layout[index].height + delta).clamp(80., 1200.);
                            }
                        }
                        cx.notify();
                    }
                }
                if let Some((origin, width)) = this.trace_resizing {
                    if event.dragging() {
                        this.prefs.trace_label_width =
                            (width + f32::from(event.position.x) - origin).clamp(140., 600.);
                        cx.notify();
                    }
                }
                if let Some((list, origin, width)) = this.resizing {
                    if event.dragging() {
                        let delta = f32::from(event.position.x) - origin;
                        let maximum = (f32::from(window.viewport_size().width) - 448.).max(160.);
                        if list {
                            this.prefs.list_width = (width + delta).clamp(160., maximum);
                        } else {
                            this.prefs.details_width = (width - delta).clamp(200., maximum);
                        }
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let trace_finished = this.trace_resizing.take().is_some();
                    let section_finished = this.detail_resizing.take().is_some();
                    if this.resizing.take().is_some() || trace_finished || section_finished {
                        this.save_preferences();
                        cx.notify();
                    }
                }),
            )
            .key_context("Viewer")
            .on_action(cx.listener(|this, _: &DismissOverlay, window, cx| {
                if this.appearance_open {
                    this.appearance_open = false;
                    this.appearance_trigger_focus.focus(window, cx);
                }
                this.time_open = false;
                this.import_report_open = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.search.read(cx).focus_handle(cx).focus(window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoHome, _, cx| {
                this.close_search_completions(cx);
                this.page = Page::Home;
                this.refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &GoTraces, _, cx| {
                this.close_search_completions(cx);
                this.page = Page::Traces;
                this.refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &GoLogs, _, cx| {
                this.close_search_completions(cx);
                this.page = Page::Logs;
                this.refresh(cx);
            }))
            .on_action(cx.listener(|this, _: &GoMetrics, _, cx| {
                this.close_search_completions(cx);
                this.page = Page::Metrics;
                this.refresh(cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(color(p.canvas))
            .text_color(color(p.text))
            .font_family("Atkinson Hyperlegible Next")
            .text_sm()
            .child(self.render_toolbar(window, cx))
            .child(div().flex_1().min_h_0().child(content))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .h(px(34.))
                    .flex_shrink_0()
                    .px_3()
                    .text_color(color(p.subtle))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .child(self.status.clone()),
                    )
                    .child(
                        self.data
                            .get("getStats")
                            .map(|stats| {
                                format!(
                                    "{} traces · {} logs · {} metrics",
                                    crate::model::field(&stats["traces"], "traceCount"),
                                    crate::model::field(&stats["logs"], "logCount"),
                                    crate::model::field(&stats["metrics"], "metricCount")
                                )
                            })
                            .unwrap_or_default(),
                    ),
            )
            .children(self.pane_resize_handles(window, cx))
            .children(
                if self.time_open || self.appearance_open || self.import_report_open {
                    Some(div().absolute().inset_0().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.time_open = false;
                            this.appearance_open = false;
                            this.import_report_open = false;
                            cx.notify();
                        }),
                    ))
                } else {
                    None
                },
            )
            .children(if self.time_open {
                Some(self.render_time_picker(cx))
            } else {
                None
            })
            .children(self.render_search_completions(window, cx))
            .children(if self.appearance_open {
                Some(self.render_appearance(cx))
            } else {
                None
            })
            .children(if self.import_report_open {
                Some(self.render_import_report(window, cx))
            } else {
                None
            })
    }
}

pub(crate) fn register_actions(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", DismissOverlay, Some("Viewer")),
        KeyBinding::new("cmd-r", Refresh, Some("Viewer")),
        KeyBinding::new("cmd-f", FocusSearch, Some("Viewer")),
        KeyBinding::new("cmd-1", GoHome, Some("Viewer")),
        KeyBinding::new("cmd-2", GoTraces, Some("Viewer")),
        KeyBinding::new("cmd-3", GoLogs, Some("Viewer")),
        KeyBinding::new("cmd-4", GoMetrics, Some("Viewer")),
    ]);
}

fn stats_received_new(previous: &Value, current: &Value) -> bool {
    ["traces", "logs", "metrics"].iter().any(|signal| {
        [
            "traceCount",
            "spanCount",
            "logCount",
            "metricCount",
            "dataPointCount",
            "lastReceived",
        ]
        .iter()
        .any(|key| {
            crate::model::ns(&current[signal][key]) > crate::model::ns(&previous[signal][key])
        })
    })
}

#[cfg(test)]
mod stats_tests {
    use super::stats_received_new;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn storage_changes_do_not_claim_new_telemetry() {
        let old =
            json!({"storage":{"sizeBytes":10},"traces":{"traceCount":2,"lastReceived":"123"}});
        let storage_only =
            json!({"storage":{"sizeBytes":15},"traces":{"traceCount":2,"lastReceived":"123"}});
        assert!(!stats_received_new(&old, &storage_only));
        assert!(stats_received_new(
            &old,
            &json!({"traces":{"traceCount":2,"lastReceived":"124"}})
        ));
        assert!(!stats_received_new(
            &old,
            &json!({"traces":{"traceCount":1,"lastReceived":"123"}})
        ));
    }
}

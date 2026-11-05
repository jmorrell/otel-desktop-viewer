use crate::{
    app::{Page, Viewer},
    model::*,
    theme::{color, palette},
    widgets,
};
use gpui_kit::component::Selectable;
use gpui_kit::{
    component::{Disableable, IconName},
    prelude::FluentBuilder,
    *,
};
use serde_json::{Value, json};
use std::{sync::mpsc, time::Duration};

const COLUMNS: [(&str, &str); 17] = [
    ("timestamp", "Timestamp"),
    ("service", "Service"),
    ("severity", "Severity"),
    ("name", "Event name"),
    ("body", "Body"),
    ("traceId", "Trace ID"),
    ("spanId", "Span ID"),
    ("scope.name", "Scope"),
    ("scope.version", "Scope version"),
    ("observedTimeUnixNano", "Observed timestamp"),
    ("severityNumber", "Severity number"),
    ("flags", "Flags"),
    ("droppedAttributesCount", "Dropped log attributes"),
    ("resource.schemaUrl", "Resource schema URL"),
    (
        "resource.droppedAttributesCount",
        "Dropped resource attributes",
    ),
    ("scope.schemaUrl", "Scope schema URL"),
    ("scope.droppedAttributesCount", "Dropped scope attributes"),
];
fn log_projection(key: &str) -> Option<Value> {
    if let Some(rest) = key.strip_prefix("attr:") {
        let (source, name) = rest.split_once(':')?;
        if ["log", "resource", "scope"].contains(&source) && !name.is_empty() {
            return Some(json!({"source": source,"kind":"attribute","key":name}));
        }
        return None;
    }
    let (source, field) = match key {
        "traceId" => ("log", "traceId"),
        "spanId" => ("log", "spanId"),
        "scope.name" => ("scope", "name"),
        "scope.version" => ("scope", "version"),
        "observedTimeUnixNano" => ("log", "observedTimeUnixNano"),
        "severityNumber" => ("log", "severityNumber"),
        "flags" => ("log", "flags"),
        "droppedAttributesCount" => ("log", "droppedAttributesCount"),
        "resource.schemaUrl" => ("resource", "schemaUrl"),
        "resource.droppedAttributesCount" => ("resource", "droppedAttributesCount"),
        "scope.schemaUrl" => ("scope", "schemaUrl"),
        "scope.droppedAttributesCount" => ("scope", "droppedAttributesCount"),
        _ => return None,
    };
    Some(json!({"source": source, "kind":"field", "key":field}))
}
fn projected_value(row: &Value, key: &str) -> String {
    let Some(projection) = log_projection(key) else {
        return String::new();
    };
    row["columnValues"]
        .as_array()
        .and_then(|v| {
            v.iter().find(|c| {
                c["source"] == projection["source"]
                    && c["kind"] == projection["kind"]
                    && c["key"] == projection["key"]
            })
        })
        .and_then(|c| c["values"].as_array())
        .map(|v| v.iter().map(attribute).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}
fn log_prefs_path() -> std::path::PathBuf {
    directories::ProjectDirs::from("dev", "OpenTelemetry", "Desktop Viewer")
        .map(|p| p.config_dir().join("logs.json"))
        .unwrap_or_else(|| "native-logs.json".into())
}
fn default_log_ui() -> Value {
    json!({"sort":"timestamp","direction":"desc","severity":"all","service":"all","columns":["severity","timestamp","service","body"],"widths":{"timestamp":220,"service":150,"severity":110,"name":150,"body":320},"checked":[],"configure":false,"confirm":""})
}
fn persisted_log_ui() -> Value {
    let mut result = default_log_ui();
    if let Ok(bytes) = std::fs::read(log_prefs_path()) {
        if let Ok(saved) = serde_json::from_slice::<Value>(&bytes) {
            for key in ["sort", "direction", "columns", "widths"] {
                if !saved[key].is_null() {
                    result[key] = saved[key].clone();
                }
            }
        }
    }
    result
}
fn save_log_ui(ui: &Value) {
    let path = log_prefs_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let persisted = json!({"sort":ui["sort"],"direction":ui["direction"],"columns":ui["columns"],"widths":ui["widths"]});
    if let Ok(bytes) = serde_json::to_vec(&persisted) {
        let _ = std::fs::write(path, bytes);
    }
}
impl Viewer {
    fn log_ui(&mut self) -> Value {
        if !self.data.contains_key("logs.ui") {
            self.data.insert(
                "logs.ui".into(),
                if self.persist_preferences {
                    persisted_log_ui()
                } else {
                    default_log_ui()
                },
            );
        }
        self.data["logs.ui"].clone()
    }
    fn set_log_ui(&mut self, ui: Value, persist: bool) {
        if persist && self.persist_preferences {
            save_log_ui(&ui);
        }
        self.data.insert("logs.ui".into(), ui);
    }
    fn sort_log_column(&mut self, key: &str, cx: &mut Context<Self>) {
        let mut ui = self.log_ui();
        let direction = if ui["sort"] == key && ui["direction"] == "desc" {
            "asc"
        } else {
            "desc"
        };
        ui["sort"] = json!(key);
        ui["direction"] = json!(direction);
        self.set_log_ui(ui, true);
        self.logs_offset = 0;
        self.refresh_logs(cx);
    }
    fn log_params(&mut self, cx: &App) -> Value {
        let ui = self.log_ui();
        let mut p = self.current_params(cx);
        let mut terms = Vec::new();
        if !p["query"].is_null() {
            terms.push(p["query"].clone());
        }
        let severity = ui["severity"].as_str().unwrap_or("all");
        if severity != "all" {
            let (op, value) = match severity {
                "error" => (">=", "17"),
                "warn" => (">=", "13"),
                _ => (">=", "9"),
            };
            terms.push(condition(
                json!({"searchScope":"field","name":"severityNumber","type":"int64"}),
                op,
                value,
            ));
        }
        if let Some(identity) = ui["service"].as_str().filter(|s| *s != "all") {
            terms.push(condition(
                json!({"searchScope":"field","name":"serviceIdentity","type":"string"}),
                "=",
                identity,
            ));
        }
        p["query"] = if terms.is_empty() {
            Value::Null
        } else if terms.len() == 1 {
            terms.remove(0)
        } else {
            json!({"id":"log-filters","type":"group","group":{"logicalOperator":"AND","children":terms}})
        };
        p["sort"] = json!({"field":ui["sort"],"direction":ui["direction"]});
        p
    }
    pub fn refresh_logs(&mut self, cx: &mut Context<Self>) {
        let mut p = self.log_params(cx);
        p["pageSize"] = json!(self.page_size);
        p["offset"] = json!(self.logs_offset);
        let ui = self.log_ui();
        let projections: Vec<Value> = ui["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().and_then(log_projection))
            .collect();
        if !projections.is_empty() {
            p["columns"] = json!(projections);
        }
        self.rpc_key("logs.page", "searchLogPage", p, cx);
        self.rpc_key("logs.services", "discoverTelemetry", json!({"options":{"kind":"services","startTime":self.time_range.start,"endTime":self.time_range.end,"limit":500}}), cx);
        self.refresh_log_navigation(cx);
    }
    fn refresh_log_navigation(&mut self, cx: &mut Context<Self>) {
        let mut p = self.log_params(cx);
        p["selectedLogRef"] = json!(self.selected_log);
        self.rpc_key("logs.navigation", "getLogErrorNavigation", p, cx);
    }
    pub fn load_log(&mut self, id: String, cx: &mut Context<Self>) {
        self.data.remove("logs.detail");
        self.rpc_key("logs.detail", "getLog", json!({"logRef":id}), cx);
        self.refresh_log_navigation(cx);
    }
    fn delete_logs(&mut self, all: bool, cx: &mut Context<Self>) {
        let mut ui = self.log_ui();
        let refs = if ui["confirm"] == "current" {
            self.selected_log
                .as_ref()
                .map(|id| vec![json!(id)])
                .unwrap_or_default()
        } else {
            ui["checked"].as_array().cloned().unwrap_or_default()
        };
        if !all && refs.is_empty() {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        ui["confirm"] = json!("");
        self.set_log_ui(ui, false);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(if all {
                backend.call("clearLogs", json!([]))
            } else {
                backend.call("deleteLogsByRefs", Value::Array(refs))
            });
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        let _ = view.update(cx, |this, cx| {
                            match result {
                                Ok(_) => {
                                    let mut ui = this.log_ui();
                                    ui["checked"] = json!([]);
                                    this.set_log_ui(ui, false);
                                    this.selected_log = None;
                                    this.data.remove("logs.detail");
                                    this.logs_offset = 0;
                                    this.refresh_logs(cx);
                                    this.status = "Logs deleted".into();
                                }
                                Err(e) => this.status = format!("Delete failed: {e:#}"),
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
    pub fn render_logs(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let ui = self.log_ui();
        let response = self.data.get("logs.page").cloned().unwrap_or(Value::Null);
        let rows = response["logs"].as_array().cloned().unwrap_or_default();
        let total = response["total"].as_u64().unwrap_or(0);
        let visible = ui["columns"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let checked = ui["checked"].as_array().cloned().unwrap_or_default();
        let mut filters = div().flex().items_center().gap_2().px_3().py_1();
        let severity = ui["severity"].as_str().unwrap_or("all").to_owned();
        filters = filters.child(
            widgets::button("log-severity")
                .label(format!("Severity: {severity}"))
                .on_click(cx.listener(|this, _, _, cx| {
                    let mut ui = this.log_ui();
                    ui["severity"] = json!(match ui["severity"].as_str().unwrap_or("all") {
                        "all" => "error",
                        "error" => "warn",
                        "warn" => "info",
                        _ => "all",
                    });
                    this.set_log_ui(ui, false);
                    this.logs_offset = 0;
                    this.refresh_logs(cx);
                })),
        );
        let controls = div()
            .flex()
            .items_center()
            .gap_1()
            .px_3()
            .h(px(48.))
            .flex_shrink_0()
            .rounded_lg()
            .bg(color(p.panel))
            .child(format!(
                "Logs · {}–{} of {total} records",
                (self.logs_offset + 1).min(total as usize),
                (self.logs_offset + rows.len()).min(total as usize)
            ))
            .child(div().flex_1())
            .child(
                widgets::button("log-filters")
                    .icon(IconName::Settings2)
                    .tooltip("Filter logs")
                    .accessibility_label("Filter logs")
                    .toggled(ui["filters"] == true)
                    .selected(ui["filters"] == true)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut ui = this.log_ui();
                        ui["filters"] = json!(ui["filters"] != true);
                        this.set_log_ui(ui, false);
                        cx.notify();
                    })),
            )
            .child(
                widgets::button("log-columns")
                    .icon(IconName::PanelLeft)
                    .tooltip("Choose columns")
                    .accessibility_label("Choose columns")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut ui = this.log_ui();
                        ui["configure"] = json!(!ui["configure"].as_bool().unwrap_or(false));
                        if ui["configure"] == true {
                            this.rpc_key(
                                "logs.definitions",
                                "getLogAttributeDefinitions",
                                json!([]),
                                cx,
                            );
                        }
                        this.set_log_ui(ui, false);
                        cx.notify();
                    })),
            )
            .child(
                widgets::button("log-select-page")
                    .icon(IconName::Check)
                    .tooltip("Select page")
                    .accessibility_label("Select page")
                    .on_click({
                        let ids = rows
                            .iter()
                            .map(|r| json!(field(r, "logRef")))
                            .collect::<Vec<_>>();
                        cx.listener(move |this, _, _, cx| {
                            let mut ui = this.log_ui();
                            let mut selected =
                                ui["checked"].as_array().cloned().unwrap_or_default();
                            for id in &ids {
                                if !selected.contains(id) {
                                    selected.push(id.clone());
                                }
                            }
                            ui["checked"] = json!(selected);
                            this.set_log_ui(ui, false);
                            cx.notify();
                        })
                    }),
            )
            .child(
                widgets::button("log-delete-selected")
                    .icon(IconName::Delete)
                    .tooltip("Delete selected logs")
                    .when(!checked.is_empty(), |button| {
                        button.label(checked.len().to_string())
                    })
                    .accessibility_label("Delete selected logs")
                    .disabled(checked.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut ui = this.log_ui();
                        if !ui["checked"].as_array().is_none_or(|v| v.is_empty()) {
                            ui["confirm"] = json!("selected");
                            this.set_log_ui(ui, false);
                            cx.notify();
                        }
                    })),
            )
            .child(
                widgets::button("log-delete-all")
                    .icon(IconName::Delete)
                    .tooltip("Delete all logs")
                    .accessibility_label("Delete all logs")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let mut ui = this.log_ui();
                        ui["confirm"] = json!("all");
                        this.set_log_ui(ui, false);
                        cx.notify();
                    })),
            );
        let mut services: Vec<(String, String)> = self
            .data
            .get("logs.services")
            .map(discovery_rows)
            .unwrap_or_default()
            .iter()
            .filter_map(|row| {
                if ns(&row["logCount"]) == 0 {
                    return None;
                }
                let identity = field(row, "serviceIdentity");
                if identity.is_empty() {
                    return None;
                }
                let name = field(row, "serviceName");
                let namespace = field(row, "serviceNamespace");
                let name_kind = field(row, "serviceNameKind");
                let namespace_kind = field(row, "serviceNamespaceKind");
                let name = if name.is_empty() {
                    format!("(unnamed · {name_kind})")
                } else if !name_kind.is_empty() && name_kind != "string" {
                    format!("{name} · {name_kind}")
                } else {
                    name
                };
                let label = if namespace.is_empty() {
                    format!("{name} · no namespace")
                } else if !namespace_kind.is_empty() && namespace_kind != "string" {
                    format!("{namespace} / {name} · {namespace_kind}")
                } else {
                    format!("{namespace} / {name}")
                };
                Some((identity, label))
            })
            .collect();
        services.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        services.dedup_by(|a, b| a.0 == b.0);
        let identity = ui["service"].as_str().unwrap_or("all").to_owned();
        let label = services
            .iter()
            .find(|(id, _)| id == &identity)
            .map(|(_, label)| label.as_str())
            .unwrap_or("All services")
            .to_owned();
        filters = filters.child(
            widgets::button("log-service")
                .label(format!("Service: {label}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let mut u = this.log_ui();
                    let current = u["service"].as_str().unwrap_or("all");
                    let next = services
                        .iter()
                        .position(|(id, _)| id == current)
                        .and_then(|i| services.get(i + 1))
                        .map(|(id, _)| id.clone())
                        .or_else(|| {
                            if current == "all" {
                                services.first().map(|(id, _)| id.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "all".into());
                    u["service"] = json!(next);
                    this.set_log_ui(u, false);
                    this.logs_offset = 0;
                    this.refresh_logs(cx);
                })),
        );
        let mut main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(color(p.canvas))
            .rounded_lg()
            .child(controls);
        if ui["filters"] == true {
            main = main.child(filters);
        }
        if ui["confirm"] != "" {
            let all = ui["confirm"] == "all";
            main = main.child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child(if all {
                        "Delete all logs?"
                    } else {
                        "Delete selected logs?"
                    })
                    .child(
                        widgets::button("confirm-log-delete")
                            .label("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_logs(all, cx))),
                    )
                    .child(
                        widgets::button("cancel-log-delete")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut u = this.log_ui();
                                u["confirm"] = json!("");
                                this.set_log_ui(u, false);
                                cx.notify();
                            })),
                    ),
            );
        }
        if ui["configure"].as_bool() == Some(true) {
            let mut bar = div().flex().flex_wrap().gap_1().p_2();
            let mut options: Vec<(String, String)> = COLUMNS
                .iter()
                .map(|(k, l)| ((*k).to_owned(), (*l).to_owned()))
                .collect();
            if let Some(defs) = self.data.get("logs.definitions").and_then(Value::as_array) {
                for def in defs {
                    let source = field(def, "attributeScope");
                    let name = field(def, "name");
                    if ["log", "resource", "scope"].contains(&source.as_str()) && !name.is_empty() {
                        let key = format!("attr:{source}:{name}");
                        if !options.iter().any(|(existing, _)| existing == &key) {
                            options.push((key, format!("{source}: {name}")));
                        }
                    }
                }
            }
            for (key, label) in options {
                let enabled = visible.iter().any(|v| v == &key);
                let toggle_key = key.clone();
                bar = bar.child(
                    widgets::button(format!("col-{key}"))
                        .label(format!("{} {label}", if enabled { "✓" } else { "○" }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut u = this.log_ui();
                            let mut cols = u["columns"].as_array().cloned().unwrap_or_default();
                            if let Some(i) = cols.iter().position(|v| v == &toggle_key) {
                                if cols.len() > 1 {
                                    cols.remove(i);
                                }
                            } else {
                                cols.push(json!(toggle_key));
                            }
                            u["columns"] = json!(cols);
                            this.set_log_ui(u, true);
                            this.refresh_logs(cx);
                        })),
                );
                if enabled {
                    for (sign, delta) in [("−", -20), ("+", 20)] {
                        let width_key = key.clone();
                        bar = bar.child(
                            widgets::button(format!("width-{key}-{sign}"))
                                .label(sign)
                                .accessibility_label(format!(
                                    "{} width for {label}",
                                    if delta < 0 { "Decrease" } else { "Increase" }
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut u = this.log_ui();
                                    let width = u["widths"][&width_key].as_i64().unwrap_or(150);
                                    u["widths"][&width_key] = json!((width + delta).clamp(80, 600));
                                    this.set_log_ui(u, true);
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
            main = main.child(bar);
        }
        let table_width = 44.
            + visible
                .iter()
                .map(|key| ui["widths"][key].as_f64().unwrap_or(150.) as f32 + 8.)
                .sum::<f32>();
        // Fixed column coordinates share the row widths even when the grid is
        // wider than its viewport and horizontally scrolled.
        let mut column_offset = 44.;
        let mut header = div()
            .relative()
            .w(px(table_width))
            .h(px(40.))
            .flex_shrink_0()
            .text_size(px(13.))
            .bg(color(p.canvas));
        for key in &visible {
            let key = key.clone();
            let label = COLUMNS
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, l)| (*l).to_owned())
                .unwrap_or_else(|| key.strip_prefix("attr:").unwrap_or(&key).replace(':', ": "));
            let width = ui["widths"][&key].as_f64().unwrap_or(150.) as f32;
            let cell = div()
                .absolute()
                .left(px(column_offset))
                .top_0()
                .w(px(width))
                .h_full()
                .flex()
                .items_center()
                .overflow_hidden();
            column_offset += width + 8.;
            if log_projection(&key).is_some() {
                header = header.child(cell.child(label));
                continue;
            }
            let marker = if ui["sort"] == key {
                if ui["direction"] == "asc" {
                    " ↑"
                } else {
                    " ↓"
                }
            } else {
                ""
            };
            // Use the unstyled native button so clipped column headers keep
            // their text at the column origin while retaining keyboard/a11y behavior.
            header = header.child(
                cell.child(
                    gpui_kit::base::Button::new(format!("sort-{key}"))
                        .accessibility_label(format!("Sort by {label}{marker}"))
                        .w(px(width))
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .justify_start()
                        .text_size(px(13.))
                        .text_ellipsis()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(color(p.selection)))
                        .focus_visible(|style| style.border_1().border_color(color(p.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sort_log_column(&key, cx);
                        }))
                        .child(format!("{label}{marker}")),
                ),
            );
        }
        let mut table = div().id("log-table").flex_1().min_h_0().overflow_y_scroll();
        for (index, row) in rows.iter().enumerate() {
            let id = field(row, "logRef");
            let selected = self.selected_log.as_ref() == Some(&id);
            let severity = ns(&row["severityNumber"]);
            let checked_row = checked.contains(&json!(id));
            let check_id = id.clone();
            let check_label = format!(
                "{} log from {} at {}",
                if checked_row { "Deselect" } else { "Select" },
                field(row, "serviceName"),
                format_timestamp(ns(&row["timestamp"]))
            );
            let mut line = div()
                .id(("log-row", index))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .h(px(40.))
                .rounded_lg()
                .bg(color(if selected { p.selection } else { p.canvas }));
            line = line.child(
                div().w(px(28.)).flex_shrink_0().child(
                    widgets::button(format!("check-log-{index}"))
                        .icon(if checked_row {
                            IconName::CircleCheck
                        } else {
                            IconName::Square
                        })
                        .w(px(24.))
                        .text_size(px(13.))
                        .accessibility_label(check_label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut u = this.log_ui();
                            let mut a = u["checked"].as_array().cloned().unwrap_or_default();
                            if let Some(i) = a.iter().position(|v| v == &check_id) {
                                a.remove(i);
                            } else {
                                a.push(json!(check_id));
                            }
                            u["checked"] = json!(a);
                            this.set_log_ui(u, false);
                            cx.notify();
                        })),
                ),
            );
            for key in &visible {
                let value = match key.as_str() {
                    "timestamp" => format_timestamp(ns(&row["timestamp"])),
                    "service" => field(row, "serviceName"),
                    "severity" => field(row, "severityText"),
                    "name" => field(row, "eventName"),
                    "body" => field(row, "bodyPreview"),
                    _ => projected_value(row, key),
                };
                let width = ui["widths"][key].as_f64().unwrap_or(150.) as f32;
                let select_id = id.clone();
                // Buttons provide native focus and keyboard activation for each
                // cell, while retaining stable IDs for headless interaction tests.
                let accessible = if value.is_empty() {
                    "(empty)".to_owned()
                } else {
                    value
                };
                line = line.child(
                    div()
                        .w(px(width))
                        .flex_shrink_0()
                        .overflow_hidden()
                        .text_color(color(if key == "severity" && severity >= 17 {
                            p.error
                        } else {
                            p.subtle
                        }))
                        .child(
                            widgets::button(format!("cell-{index}-{key}"))
                                .accessibility_label(format!("Inspect log, {key}: {accessible}"))
                                .child(
                                    div()
                                        .w_full()
                                        .text_ellipsis()
                                        .text_color(color(if key == "severity" && severity >= 17 {
                                            p.error
                                        } else {
                                            p.text
                                        }))
                                        .child(accessible),
                                )
                                .text_size(px(13.))
                                .w_full()
                                .justify_start()
                                .px_0()
                                .overflow_hidden()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_log(select_id.clone(), cx)
                                })),
                        ),
                );
            }
            table = table.child(line);
        }
        if rows.is_empty() {
            table = table.child(widgets::empty("No logs in this time range", p));
        }
        let nav = self
            .data
            .get("logs.navigation")
            .cloned()
            .unwrap_or(Value::Null);
        let mut footer = div().flex().justify_between().items_center().p_2().gap_1();
        footer = footer
            .child(
                widgets::button("logs-prev")
                    .icon(IconName::ChevronLeft)
                    .tooltip("Previous page")
                    .accessibility_label("Previous page")
                    .disabled(self.logs_offset == 0)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.logs_offset = this.logs_offset.saturating_sub(this.page_size);
                        this.refresh_logs(cx);
                    })),
            )
            .child(format!(
                "{}–{} / {total}",
                (self.logs_offset + 1).min(total as usize),
                (self.logs_offset + rows.len()).min(total as usize)
            ))
            .child(
                widgets::button("logs-next")
                    .icon(IconName::ChevronRight)
                    .tooltip("Next page")
                    .accessibility_label("Next page")
                    .disabled(self.logs_offset + self.page_size >= total as usize)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.logs_offset + this.page_size < total as usize {
                            this.logs_offset += this.page_size;
                            this.refresh_logs(cx);
                        }
                    })),
            );
        footer = footer.child(
            widgets::button("log-page-size")
                .label(format!("{} / page", self.page_size))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.page_size = match this.page_size {
                        25 => 50,
                        50 => 100,
                        _ => 25,
                    };
                    this.logs_offset = 0;
                    this.refresh_logs(cx);
                })),
        );
        for (key, label) in [("previous", "Previous error"), ("next", "Next error")] {
            if let Some(target) = nav[key].as_object() {
                let id = target
                    .get("logRef")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let offset = target.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
                footer = footer.child(
                    widgets::button(format!("log-error-{key}"))
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.logs_offset = offset / this.page_size * this.page_size;
                            this.selected_log = Some(id.clone());
                            this.page = Page::Logs;
                            this.load_log(id.clone(), cx);
                            this.refresh_logs(cx);
                        })),
                );
            }
        }
        main = main
            .child(
                div()
                    .id("log-grid")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .child(
                        div()
                            .w(px(table_width))
                            .min_w_full()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(header)
                            .child(table),
                    ),
            )
            .child(footer);
        let details = if self.prefs.details_open {
            let data = self.data.get("logs.detail").cloned().unwrap_or(Value::Null);
            let mut details = div()
                .id("log-details")
                .size_full()
                .overflow_y_scroll()
                .bg(color(p.canvas))
                .rounded_lg();
            if !data.is_null() {
                let copy = serde_json::to_string_pretty(&data).unwrap_or_default();
                let reference = self.selected_log.clone().unwrap_or_default();
                let delete_ref = reference.clone();
                let copy_otlp_ref = reference.clone();
                let json_mode = self
                    .data
                    .get("ui.log.json")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let actions = div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        widgets::button("log-properties")
                            .label("Properties")
                            .toggled(!json_mode)
                            .selected(!json_mode)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.remove("ui.log.json");
                                cx.notify();
                            })),
                    )
                    .child(
                        widgets::button("log-json-view")
                            .label("JSON")
                            .toggled(json_mode)
                            .selected(json_mode)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.insert("ui.log.json".into(), json!(true));
                                cx.notify();
                            })),
                    )
                    .child(
                        widgets::button("copy-log")
                            .icon(IconName::Copy)
                            .w(px(24.))
                            .tooltip("Copy log JSON")
                            .accessibility_label("Copy log JSON")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    )
                    .child(
                        widgets::button("copy-log-otlp")
                            .icon(IconName::FileText)
                            .w(px(24.))
                            .tooltip("Copy OTLP JSON")
                            .accessibility_label("Copy OTLP JSON")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_otlp("logs", copy_otlp_ref.clone(), None, cx)
                            })),
                    )
                    .child(
                        widgets::button("export-log")
                            .icon(IconName::ArrowDown)
                            .w(px(24.))
                            .tooltip("Export log")
                            .accessibility_label("Export log")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.export_record("logs", reference.clone(), None, cx)
                            })),
                    )
                    .child(
                        widgets::button("delete-current-log")
                            .icon(IconName::Delete)
                            .w(px(24.))
                            .tooltip("Delete log")
                            .accessibility_label("Delete log")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut ui = this.log_ui();
                                if delete_ref.is_empty() {
                                    return;
                                }
                                ui["confirm"] = json!("current");
                                this.set_log_ui(ui, false);
                                cx.notify();
                            })),
                    );
                let mut summary = div().flex().flex_col();
                for (key, label) in [
                    ("timestamp", "timeUnixNano · fixed64"),
                    ("observedTimestamp", "observedTimeUnixNano · fixed64"),
                ] {
                    summary = summary.child(widgets::property_exact(
                        label,
                        format!(
                            "{}\n{}",
                            format_timestamp(ns(&data[key])),
                            field(&data, key)
                        ),
                        field(&data, key),
                        p,
                    ));
                }
                for (key, label) in [
                    ("severityText", "severityText · string"),
                    ("severityNumber", "severityNumber · enum"),
                    ("eventName", "eventName · string"),
                    ("traceID", "traceId · bytes"),
                    ("spanID", "spanId · bytes"),
                    ("flags", "flags · uint32"),
                    ("droppedAttributesCount", "droppedAttributesCount · uint32"),
                ] {
                    summary = summary.child(widgets::property(label, field(&data, key), p));
                }
                summary = summary.child(widgets::property(
                    format!("body · {}", field(&data["body"], "kind")),
                    attribute(&data["body"]),
                    p,
                ));
                if let Some(trace) = data["traceID"].as_str().filter(|s| !s.is_empty()) {
                    let trace = trace.to_owned();
                    let span = data["spanID"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.chars().any(|c| c != '0'))
                        .map(str::to_owned);
                    summary = summary.child(
                        widgets::button("log-open-trace")
                            .label("Open associated trace")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_trace(trace.clone(), cx);
                                if let Some(span) = &span {
                                    this.select_span(span.clone(), cx);
                                }
                            })),
                    );
                }
                for (key, label) in [
                    ("attributes", "Attributes"),
                    ("resource", "Resource"),
                    ("scope", "Scope"),
                ] {
                    summary = summary
                        .child(widgets::heading(label, p))
                        .child(self.attribute_table(&format!("log.{key}"), &data[key], cx));
                }
                let raw = self.raw_otlp_inspector(
                    "log",
                    "logs",
                    self.selected_log.clone().unwrap_or_default(),
                    None,
                    cx,
                );
                summary = summary.child(raw);
                let primary = if json_mode {
                    self.detail_json("log-json", &data, 0, cx)
                } else {
                    summary.into_any_element()
                };
                let sections = vec![
                    crate::detail_stack::DetailSection::new("summary", "Log", primary)
                        .actions(actions),
                ];
                details = details.child(self.detail_stack("logs", sections, cx));
            } else {
                details = details.child(widgets::empty(
                    "Select a log to inspect its fields and associated trace",
                    p,
                ));
            }
            Some(details.into_any_element())
        } else {
            None
        };
        self.render_workspace(None, main.into_any_element(), details, window)
    }
}

#[cfg(test)]
mod tests {
    use super::projected_value;
    use serde_json::json;
    #[test]
    fn projected_columns_preserve_typed_values() {
        let row = json!({"columnValues":[
            {"source":"scope","kind":"field","key":"name","values":[{"kind":"string","value":"checkout"}]},
            {"source":"resource","kind":"attribute","key":"service.name","values":[{"kind":"string","value":"payment-service"}]},
            {"source":"log","kind":"field","key":"traceId","values":[{"kind":"string","value":"0123456789abcdef0123456789abcdef"}]}
        ]});
        assert_eq!(projected_value(&row, "scope.name"), "checkout");
        assert_eq!(
            projected_value(&row, "traceId"),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(projected_value(&row, "spanId"), "");
        assert_eq!(
            projected_value(&row, "attr:resource:service.name"),
            "payment-service"
        );
    }
}

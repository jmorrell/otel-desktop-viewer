use crate::{
    app::{Page, Viewer},
    model::*,
    theme::{color, palette},
    widgets,
};
use gpui_kit::{
    component::{Disableable, button::Button},
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
    json!({"sort":"timestamp","direction":"desc","severity":"all","service":"all","columns":["timestamp","service","severity","body"],"widths":{"timestamp":220,"service":150,"severity":110,"name":150,"body":320},"checked":[],"configure":false,"confirm":""})
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
            self.data.insert("logs.ui".into(), persisted_log_ui());
        }
        self.data["logs.ui"].clone()
    }
    fn set_log_ui(&mut self, ui: Value, persist: bool) {
        if persist {
            save_log_ui(&ui);
        }
        self.data.insert("logs.ui".into(), ui);
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
        let (_, details_width) = self.pane_widths(window);
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
        let mut controls = div().flex().items_center().gap_1().p_2().flex_wrap();
        let severity = ui["severity"].as_str().unwrap_or("all").to_owned();
        controls = controls
            .child(
                Button::new("log-severity")
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
            )
            .child(
                Button::new("log-columns")
                    .label("Columns")
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
                Button::new("log-select-page")
                    .label("Select page")
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
                Button::new("log-delete-selected")
                    .label(format!("Delete selected ({})", checked.len()))
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
                Button::new("log-delete-all")
                    .label("Delete all")
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
        controls = controls.child(
            Button::new("log-service")
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
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(widgets::heading(format!("Logs · {total}"), p)),
            )
            .child(controls);
        if ui["confirm"] != "" {
            let all = ui["confirm"] == "all";
            main =
                main.child(
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
                            Button::new("confirm-log-delete").label("Delete").on_click(
                                cx.listener(move |this, _, _, cx| this.delete_logs(all, cx)),
                            ),
                        )
                        .child(Button::new("cancel-log-delete").label("Cancel").on_click(
                            cx.listener(|this, _, _, cx| {
                                let mut u = this.log_ui();
                                u["confirm"] = json!("");
                                this.set_log_ui(u, false);
                                cx.notify();
                            }),
                        )),
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
                    Button::new(format!("col-{key}"))
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
                            Button::new(format!("width-{key}-{sign}"))
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
        let mut header = div()
            .flex()
            .px_2()
            .py_2()
            .gap_2()
            .bg(color(p.chrome))
            .child(div().w(px(28.)).child("✓"));
        for key in &visible {
            let key = key.clone();
            let label = COLUMNS
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, l)| (*l).to_owned())
                .unwrap_or_else(|| key.strip_prefix("attr:").unwrap_or(&key).replace(':', ": "));
            let width = ui["widths"][&key].as_f64().unwrap_or(150.) as f32;
            if log_projection(&key).is_some() {
                header = header.child(div().w(px(width)).child(label));
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
            header = header.child(
                div().w(px(width)).child(
                    Button::new(format!("sort-{key}"))
                        .label(format!("{label}{marker}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let mut u = this.log_ui();
                            let dir = if u["sort"] == key && u["direction"] == "desc" {
                                "asc"
                            } else {
                                "desc"
                            };
                            u["sort"] = json!(key);
                            u["direction"] = json!(dir);
                            this.set_log_ui(u, true);
                            this.logs_offset = 0;
                            this.refresh_logs(cx);
                        })),
                ),
            );
        }
        main = main.child(header);
        let mut table = div().id("log-table").flex_1().overflow_y_scroll();
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
                .gap_2()
                .px_2()
                .py_2()
                .bg(color(if selected { p.selection } else { p.canvas }))
                .border_b_1()
                .border_color(color(p.chrome));
            line = line.child(
                div().w(px(28.)).child(
                    Button::new(format!("check-log-{index}"))
                        .label(if checked_row { "☑" } else { "☐" })
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
                        .overflow_hidden()
                        .text_color(color(if key == "severity" && severity >= 17 {
                            p.error
                        } else {
                            p.subtle
                        }))
                        .child(
                            Button::new(format!("cell-{index}-{key}"))
                                .accessibility_label(format!("Inspect log, {key}: {accessible}"))
                                .label(accessible)
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
                Button::new("logs-prev")
                    .label("Previous page")
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
                Button::new("logs-next")
                    .label("Next page")
                    .disabled(self.logs_offset + self.page_size >= total as usize)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.logs_offset + this.page_size < total as usize {
                            this.logs_offset += this.page_size;
                            this.refresh_logs(cx);
                        }
                    })),
            );
        footer = footer.child(
            Button::new("log-page-size")
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
                    Button::new(format!("log-error-{key}"))
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
        main = main.child(table).child(footer);
        let mut shell = div().flex().size_full().gap_2().child(main);
        if self.prefs.details_open {
            let data = self.data.get("logs.detail").cloned().unwrap_or(Value::Null);
            let mut details = div()
                .id("log-details")
                .w(px(details_width))
                .h_full()
                .overflow_y_scroll()
                .bg(color(p.panel))
                .rounded_lg()
                .child(widgets::heading("Log details", p));
            if !data.is_null() {
                let copy = serde_json::to_string_pretty(&data).unwrap_or_default();
                let reference = self.selected_log.clone().unwrap_or_default();
                let delete_ref = reference.clone();
                let copy_otlp_ref = reference.clone();
                details = details.child(
                    div()
                        .flex()
                        .gap_1()
                        .p_2()
                        .child(Button::new("copy-log").label("Copy JSON").on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            },
                        ))
                        .child(
                            Button::new("copy-log-otlp")
                                .label("Copy OTLP JSON")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_otlp("logs", copy_otlp_ref.clone(), None, cx)
                                })),
                        )
                        .child(
                            Button::new("export-log")
                                .label("Export")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.export_record("logs", reference.clone(), None, cx)
                                })),
                        )
                        .child(Button::new("delete-current-log").label("Delete").on_click(
                            cx.listener(move |this, _, _, cx| {
                                let mut ui = this.log_ui();
                                if delete_ref.is_empty() {
                                    return;
                                }
                                ui["confirm"] = json!("current");
                                this.set_log_ui(ui, false);
                                cx.notify();
                            }),
                        )),
                );
                let mut summary = div().flex().flex_col();
                summary = summary.child(widgets::property("Body", attribute(&data["body"]), p));
                for (key, label) in [
                    ("timestamp", "Timestamp"),
                    ("observedTimestamp", "Observed time"),
                ] {
                    summary = summary.child(widgets::property(
                        label,
                        format_timestamp(ns(&data[key])),
                        p,
                    ));
                }
                for (key, label) in [
                    ("severityText", "Severity"),
                    ("severityNumber", "Severity number"),
                    ("eventName", "Event name"),
                    ("traceID", "Trace ID"),
                    ("spanID", "Span ID"),
                    ("flags", "Flags"),
                    ("droppedAttributesCount", "Dropped attributes"),
                ] {
                    summary = summary.child(widgets::property(label, field(&data, key), p));
                }
                if let Some(trace) = data["traceID"].as_str().filter(|s| !s.is_empty()) {
                    let trace = trace.to_owned();
                    let span = data["spanID"]
                        .as_str()
                        .filter(|s| !s.is_empty() && s.chars().any(|c| c != '0'))
                        .map(str::to_owned);
                    summary = summary.child(
                        Button::new("log-open-trace")
                            .label("Open associated trace")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_trace(trace.clone(), cx);
                                if let Some(span) = &span {
                                    this.select_span(span.clone(), cx);
                                }
                            })),
                    );
                }
                let mut sections = vec![crate::detail_stack::DetailSection::new(
                    "summary", "Log", summary,
                )];
                for (key, label) in [
                    ("attributes", "Attributes"),
                    ("resource", "Resource"),
                    ("scope", "Scope"),
                    ("body", "Received body"),
                ] {
                    let content = self.detail_json(&format!("log.{key}"), &data[key], 0, cx);
                    sections.push(crate::detail_stack::DetailSection::new(key, label, content));
                }
                let raw = self.raw_otlp_inspector(
                    "log",
                    "logs",
                    self.selected_log.clone().unwrap_or_default(),
                    None,
                    cx,
                );
                sections.push(crate::detail_stack::DetailSection::new(
                    "otlp",
                    "Raw OTLP JSON",
                    raw,
                ));
                details = details.child(self.detail_stack("logs", sections, cx));
            } else {
                details = details.child(widgets::empty(
                    "Select a log to inspect its fields and associated trace",
                    p,
                ));
            }
            shell = shell.child(details);
        }
        shell.into_any_element()
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

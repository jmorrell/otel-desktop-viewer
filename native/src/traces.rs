use crate::{
    app::Viewer,
    model::{field, format_duration, format_timestamp, ns, service},
    theme::{color, palette},
    widgets,
};
use gpui_kit::{
    component::{Disableable, button::Button},
    *,
};
use serde_json::{Value, json};
actions!(
    trace_workspace,
    [PreviousSpan, NextSpan, CollapseSpan, ExpandSpan]
);
impl Viewer {
    pub fn render_traces(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let (list_width, details_width) = self.pane_widths(window);
        let available = f32::from(window.viewport_size().width)
            - if self.prefs.list_open { list_width } else { 0. }
            - if self.prefs.details_open {
                details_width
            } else {
                0.
            };
        self.trace_label_render_width = self
            .prefs
            .trace_label_width
            .clamp(140., (available - 120.).clamp(140., 600.));
        let mut shell = div()
            .id("trace-workspace")
            .test_support()
            .track_focus(&self.trace_focus)
            .key_context("TraceWorkspace")
            .on_action(cx.listener(|this, _: &PreviousSpan, _, cx| this.navigate_span(false, cx)))
            .on_action(cx.listener(|this, _: &NextSpan, _, cx| this.navigate_span(true, cx)))
            .on_action(cx.listener(|this, _: &CollapseSpan, _, cx| {
                if let Some(id) = this.selected_span.clone() {
                    this.collapsed.insert(this.span_collapse_key(&id));
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ExpandSpan, _, cx| {
                if let Some(id) = this.selected_span.clone() {
                    this.collapsed.remove(&this.span_collapse_key(&id));
                    cx.notify();
                }
            }))
            .flex()
            .size_full()
            .gap_1();
        if self.prefs.list_open {
            let mut list = div()
                .id("trace-list")
                .test_support()
                .w(px(list_width))
                .h_full()
                .flex()
                .flex_col()
                .bg(color(p.panel))
                .child(widgets::heading("Traces", p))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .px_2()
                        .child(
                            Button::new("trace-sort")
                                .label(
                                    [
                                        "Start time",
                                        "Duration",
                                        "Root span name",
                                        "Service name",
                                        "Span count",
                                        "Error count",
                                    ][(self.trace_sort / 2) % 6],
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.trace_sort = (this.trace_sort + 2) % 12;
                                    this.refresh(cx);
                                })),
                        )
                        .child(
                            Button::new("trace-sort-direction")
                                .label(if self.trace_sort % 2 == 0 {
                                    "↓"
                                } else {
                                    "↑"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.trace_sort ^= 1;
                                    this.refresh(cx);
                                })),
                        ),
                );
            let mut cards = div().id("trace-cards").overflow_y_scroll().flex_1();
            let data = self
                .data
                .get("searchTraceSummaries")
                .cloned()
                .unwrap_or(Value::Null);
            let rows = data
                .as_array()
                .or_else(|| data.get("results").and_then(Value::as_array));
            if let Some(rows) = rows {
                for row in rows {
                    let id = field(row, "traceID");
                    let selected = self.selected_trace.as_ref() == Some(&id);
                    let name = row
                        .get("rootSpan")
                        .map(|r| field(r, "name"))
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| "[missing root span]".into());
                    let service_name = row
                        .get("rootSpan")
                        .map(|r| field(r, "serviceName"))
                        .unwrap_or_default();
                    let key_id = id.clone();
                    let focus = self
                        .trace_card_focus
                        .entry(id.clone())
                        .or_insert_with(|| cx.focus_handle())
                        .clone();
                    cards = cards.child(
                        div()
                            .id(SharedString::from(id.clone()))
                            .test_support()
                            .role(Role::Button)
                            .aria_selected(selected)
                            .track_focus(&focus)
                            .tab_stop(true)
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if ["enter", "space"].contains(&event.keystroke.key.as_str()) {
                                    this.select_trace(key_id.clone(), cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .min_w_0()
                            .overflow_hidden()
                            .p_3()
                            .mb_1()
                            .rounded_md()
                            .bg(color(if selected { p.selection } else { p.canvas }))
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.select_trace(id.clone(), cx)
                                }),
                            )
                            .child(div().text_ellipsis().child(name))
                            .child(
                                div()
                                    .text_ellipsis()
                                    .text_color(color(p.subtle))
                                    .child(service_name),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .child(format_timestamp(ns(&row["startTime"]))),
                                    )
                                    .child(format_duration(ns(&row["durationNs"]))),
                            )
                            .child(
                                div()
                                    .text_color(color(if ns(&row["errorCount"]) > 0 {
                                        p.error
                                    } else {
                                        p.subtle
                                    }))
                                    .child(format!(
                                        "{} spans · {} errors",
                                        field(row, "spanCount"),
                                        field(row, "errorCount")
                                    )),
                            ),
                    );
                }
            } else {
                cards = cards.child(widgets::empty("No traces in this time range", p));
            }
            list = list.child(cards);
            shell = shell.child(list);
        }
        let mut main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(color(p.canvas));
        let spans = self
            .trace_view
            .get("spans")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.trace_parent_ids = spans
            .iter()
            .map(|n| field(&n["spanData"], "parentSpanID"))
            .filter(|id| !id.is_empty())
            .collect();
        let title = self
            .selected_trace
            .clone()
            .unwrap_or_else(|| "Select a trace".into());
        main = main.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(widgets::heading(title, p))
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .child(Button::new("expand-spans").label("Expand all").on_click(
                            cx.listener(|this, _, _, cx| {
                                let prefix = format!(
                                    "span:{}:",
                                    this.selected_trace.as_deref().unwrap_or_default()
                                );
                                this.collapsed.retain(|key| !key.starts_with(&prefix));
                                cx.notify();
                            }),
                        ))
                        .child(
                            Button::new("collapse-spans")
                                .label("Collapse all")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(spans) = this.trace_view["spans"].as_array() {
                                        for node in spans {
                                            this.collapsed.insert(this.span_collapse_key(&field(
                                                &node["spanData"],
                                                "spanID",
                                            )));
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        );
        if let Some(trace_id) = self.selected_trace.clone() {
            let copy_trace_id = trace_id.clone();
            let export_id = trace_id.clone();
            let mut actions = div().flex().gap_1().px_2().child(
                Button::new("export-trace")
                    .label("Export trace")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.export_record("traces", export_id.clone(), None, cx)
                    })),
            );
            actions = actions.child(Button::new("copy-trace-otlp").label("Copy OTLP").on_click(
                cx.listener(move |this, _, _, cx| {
                    this.copy_otlp("traces", copy_trace_id.clone(), None, cx)
                }),
            ));
            if self
                .data
                .get("ui.trace.delete")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                actions = actions
                    .child(
                        div()
                            .text_color(color(p.error))
                            .child("Delete this trace and all its spans?"),
                    )
                    .child(
                        Button::new("confirm-trace-delete")
                            .label("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.rpc_key(
                                    "trace.delete",
                                    "deleteSpansByTraceID",
                                    json!([trace_id]),
                                    cx,
                                )
                            })),
                    )
                    .child(Button::new("cancel-trace-delete").label("Cancel").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.data.remove("ui.trace.delete");
                            cx.notify();
                        }),
                    ));
            } else {
                actions =
                    actions.child(Button::new("delete-trace").label("Delete trace").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.data.insert("ui.trace.delete".into(), json!(true));
                            cx.notify();
                        }),
                    ));
            }
            main = main.child(actions);
        }
        let unplaced = ns(&self.trace_view["unplacedSpanCount"]);
        if unplaced > 0 {
            main=main.child(div().px_3().py_2().text_color(color(p.error)).child(format!("{unplaced} spans could not be placed in the hierarchy; inspect raw data for malformed parent links.")));
        }
        main =
            main.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .child(
                        Button::new("previous-span-error")
                            .label("Previous error")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.navigate_span_error(false, cx)),
                            ),
                    )
                    .child(Button::new("next-span-error").label("Next error").on_click(
                        cx.listener(|this, _, _, cx| this.navigate_span_error(true, cx)),
                    )),
            );
        main = main.child(
            Button::new("trace-matches")
                .label(if self.trace_matches_only {
                    "Show all spans"
                } else {
                    "Show matches and ancestors"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.trace_matches_only = !this.trace_matches_only;
                    cx.notify();
                })),
        );
        let duration = spans
            .iter()
            .map(|n| ns(&n["spanData"]["start"]) + ns(&n["spanData"]["dur"]))
            .max()
            .unwrap_or(1)
            .max(1);
        main = main.child(
            div()
                .flex()
                .h(px(28.))
                .border_b_1()
                .border_color(color(p.chrome))
                .child(
                    div()
                        .relative()
                        .w(px(self.trace_label_render_width))
                        .flex_shrink_0()
                        .px_3()
                        .child("Service / span")
                        .child(
                            div()
                                .id("trace-column-resize")
                                .test_support()
                                .absolute()
                                .right(px(-3.))
                                .top_0()
                                .w(px(7.))
                                .h_full()
                                .cursor_col_resize()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                        this.trace_resizing = Some((
                                            f32::from(event.position.x),
                                            this.prefs.trace_label_width,
                                        ));
                                        cx.stop_propagation();
                                    }),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .justify_between()
                        .child("0")
                        .child(format_duration(duration / 2))
                        .child(format_duration(duration)),
                ),
        );
        let matched = if self.trace_matches_only {
            Some(matching_ancestors(&spans))
        } else {
            None
        };
        let mut visible = Vec::new();
        let mut hidden_depth = None;
        for node in &spans {
            let depth = node["depth"].as_u64().unwrap_or(0) as usize;
            if let Some(hidden) = hidden_depth {
                if depth > hidden {
                    continue;
                }
                hidden_depth = None;
            }
            if matched
                .as_ref()
                .is_some_and(|ids| !ids.contains(&field(&node["spanData"], "spanID")))
            {
                continue;
            }
            visible.push(node.clone());
            if self
                .collapsed
                .contains(&self.span_collapse_key(&field(&node["spanData"], "spanID")))
            {
                hidden_depth = Some(depth);
            }
        }
        self.data.insert(
            "ui.trace.visible".into(),
            json!(
                visible
                    .iter()
                    .map(|n| field(&n["spanData"], "spanID"))
                    .collect::<Vec<_>>()
            ),
        );
        if self.trace_reveal {
            if let Some(index) = visible
                .iter()
                .position(|n| self.selected_span.as_deref() == n["spanData"]["spanID"].as_str())
            {
                self.trace_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
                self.trace_reveal = false;
            }
        }
        if visible.is_empty() {
            main = main.child(widgets::empty(
                "Select a trace to inspect its span waterfall",
                p,
            ));
        } else {
            main = main.child(
                uniform_list(
                    "waterfall",
                    visible.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| this.render_span_row(&visible[index], duration, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .w_full()
                .track_scroll(&self.trace_scroll)
                .flex_1(),
            );
        }
        shell = shell.child(main);
        if self.prefs.details_open {
            let mut details = div()
                .id("span-details")
                .test_support()
                .w(px(details_width))
                .h_full()
                .overflow_y_scroll()
                .bg(color(p.panel))
                .child(widgets::heading("Span details", p));
            let selected = spans
                .iter()
                .find(|n| self.selected_span.as_deref() == n["spanData"]["spanID"].as_str())
                .or_else(|| spans.first());
            if let Some(node) = selected {
                let span = &node["spanData"];
                let mut exact = span.clone();
                if let Some(obj) = exact.as_object_mut() {
                    obj.insert("traceID".into(), self.trace_view["traceID"].clone());
                    let start = ns(&self.trace_view["traceStart"]) + ns(&span["start"]);
                    obj.insert("startTime".into(), json!(start.to_string()));
                    obj.insert(
                        "endTime".into(),
                        json!((start + ns(&span["dur"])).to_string()),
                    );
                    for key in ["r", "s", "start", "dur"] {
                        obj.remove(key);
                    }

                    obj.insert(
                        "resource".into(),
                        self.trace_view["resources"][field(span, "r")].clone(),
                    );
                    obj.insert(
                        "scope".into(),
                        self.trace_view["scopes"][field(span, "s")].clone(),
                    );
                }
                let copy = serde_json::to_string_pretty(&exact).unwrap_or_default();
                let trace_id = self.selected_trace.clone().unwrap_or_default();
                let span_id = field(span, "spanID");
                details =
                    details.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .p_2()
                            .child(Button::new("copy-span").label("Copy JSON").on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                },
                            ))
                            .child({
                                let trace_id = trace_id.clone();
                                let span_id = span_id.clone();
                                Button::new("copy-span-otlp").label("Copy OTLP").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.copy_otlp(
                                            "traces",
                                            trace_id.clone(),
                                            Some(json!({"spanId":span_id})),
                                            cx,
                                        )
                                    }),
                                )
                            })
                            .child(Button::new("export-span").label("Export").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.export_record(
                                        "traces",
                                        trace_id.clone(),
                                        Some(json!({"spanId":span_id})),
                                        cx,
                                    )
                                }),
                            )),
                    );
                if let Some(event) = self.data.get("ui.trace.event").cloned() {
                    details = details
                        .child(widgets::heading("Selected event", p))
                        .child(self.detail_json("span/selected-event", &event, 0, cx));
                }
                let mut sections = Vec::new();
                let mut summary = div();
                for (key, label) in [
                    ("name", "Name"),
                    ("spanID", "Span ID"),
                    ("parentSpanID", "Parent span"),
                    ("kind", "Kind"),
                    ("statusCode", "Status"),
                    ("statusMessage", "Status message"),
                    ("traceState", "Trace state"),
                    ("flags", "Flags"),
                    ("droppedAttributesCount", "Dropped attributes"),
                    ("droppedEventsCount", "Dropped events"),
                    ("droppedLinksCount", "Dropped links"),
                ] {
                    summary = summary.child(widgets::property(label, field(span, key), p));
                }
                summary = summary.child(widgets::property(
                    "Trace ID",
                    field(&self.trace_view, "traceID"),
                    p,
                ));
                for (key, label) in [("startTime", "Start time"), ("endTime", "End time")] {
                    summary = summary.child(widgets::property_exact(
                        label,
                        format!(
                            "{} · {} ns",
                            format_timestamp(ns(&exact[key])),
                            field(&exact, key)
                        ),
                        field(&exact, key),
                        p,
                    ));
                }
                summary = summary.child(widgets::property(
                    "Duration",
                    format_duration(ns(&span["dur"])),
                    p,
                ));
                sections.push(crate::detail_stack::DetailSection::new(
                    "summary", "Span", summary,
                ));
                for (key, label) in [
                    ("attributes", "Attributes"),
                    ("resource", "Resource"),
                    ("scope", "Scope"),
                    ("events", "Events"),
                    ("links", "Links"),
                ] {
                    let content = self.detail_json(
                        &format!("span/{}/{key}", field(span, "spanID")),
                        &exact[key],
                        0,
                        cx,
                    );
                    sections.push(crate::detail_stack::DetailSection::new(key, label, content));
                }
                if let Some(links) = span["links"].as_array() {
                    for (index, link) in links.iter().enumerate() {
                        if let Some(trace) = link["traceID"].as_str().filter(|s| !s.is_empty()) {
                            let trace = trace.to_owned();
                            details = details.child(
                                Button::new(("span-linked-trace", index))
                                    .label(format!("Open linked trace {}", trace))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_trace(trace.clone(), cx)
                                    })),
                            );
                        }
                    }
                }
                if let Some(logs) = self
                    .data
                    .get("span.logs")
                    .and_then(|v| v.get("logs").or(Some(v)))
                    .and_then(Value::as_array)
                    .cloned()
                {
                    details = details.child(widgets::heading(format!("Logs [{}]", logs.len()), p));
                    for (index, log) in logs.iter().enumerate() {
                        let id = field(log, "logRef");
                        let label = crate::model::attribute(&log["body"]);
                        details =
                            details.child(Button::new(("span-log", index)).label(label).on_click(
                                cx.listener(move |this, _, _, cx| this.select_log(id.clone(), cx)),
                            ));
                    }
                }
                let raw = self.detail_json(
                    &format!("span/{}/raw", field(span, "spanID")),
                    &exact,
                    0,
                    cx,
                );
                sections.push(crate::detail_stack::DetailSection::new(
                    "raw", "Raw JSON", raw,
                ));
                let otlp = self.raw_otlp_inspector(
                    "span",
                    "traces",
                    self.selected_trace.clone().unwrap_or_default(),
                    Some(json!({"spanId":field(span,"spanID")})),
                    cx,
                );
                sections.push(crate::detail_stack::DetailSection::new(
                    "otlp",
                    "Raw OTLP JSON",
                    otlp,
                ));
                details = details.child(self.detail_stack("traces", sections, cx));
            } else {
                details = details.child(widgets::empty("Select a span", p));
            }
            shell = shell.child(details);
        }
        shell.into_any_element()
    }
    fn span_collapse_key(&self, id: &str) -> String {
        format!(
            "span:{}:{id}",
            self.selected_trace.as_deref().unwrap_or_default()
        )
    }
    fn navigate_span(&mut self, forward: bool, cx: &mut Context<Self>) {
        let ids = self
            .data
            .get("ui.trace.visible")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if ids.is_empty() {
            return;
        }
        let current = ids
            .iter()
            .position(|id| id.as_str() == self.selected_span.as_deref());
        let next = match current {
            Some(index) if forward => (index + 1).min(ids.len() - 1),
            Some(index) => index.saturating_sub(1),
            None => 0,
        };
        if let Some(id) = ids[next].as_str() {
            self.select_span(id.to_owned(), cx);
        }
    }
    pub fn select_span(&mut self, id: String, cx: &mut Context<Self>) {
        self.data.remove("ui.trace.event");
        self.selected_span = Some(id.clone());
        self.trace_reveal = true;
        let spans = self.trace_view["spans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut parent = id.clone();
        let mut seen = std::collections::HashSet::new();
        while seen.insert(parent.clone()) {
            let Some(node) = spans
                .iter()
                .find(|n| n["spanData"]["spanID"].as_str() == Some(&parent))
            else {
                break;
            };
            parent = field(&node["spanData"], "parentSpanID");
            self.collapsed.remove(&self.span_collapse_key(&parent));
            if parent.is_empty() {
                break;
            }
        }
        if let Some(trace) = self.selected_trace.clone() {
            self.rpc_key(
                "span.logs",
                "getSpan",
                json!({"traceID":trace,"spanID":id}),
                cx,
            );
        }
        cx.notify();
    }
    fn navigate_span_error(&mut self, forward: bool, cx: &mut Context<Self>) {
        let spans = self.trace_view["spans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if spans.is_empty() {
            return;
        }
        let current = spans
            .iter()
            .position(|n| self.selected_span.as_deref() == n["spanData"]["spanID"].as_str())
            .unwrap_or(if forward { spans.len() - 1 } else { 0 });
        for step in 1..=spans.len() {
            let index = if forward {
                (current + step) % spans.len()
            } else {
                (current + spans.len() - step) % spans.len()
            };
            if ns(&spans[index]["spanData"]["statusCodeValue"]) == 2 {
                self.select_span(field(&spans[index]["spanData"], "spanID"), cx);
                break;
            }
        }
    }
    fn render_span_row(
        &mut self,
        node: &Value,
        duration: i128,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let depth = node["depth"].as_u64().unwrap_or(0) as usize;
        let span = &node["spanData"];
        let id = field(span, "spanID");
        let collapsed = self.collapsed.contains(&self.span_collapse_key(&id));
        let selected = self.selected_span.as_ref() == Some(&id);
        let error = ns(&span["statusCodeValue"]) == 2;
        let start = ns(&span["start"]);
        let dur = ns(&span["dur"]);
        let (offset, width) = timing_geometry(start, dur, duration);
        let resource = &self.trace_view["resources"][field(span, "r")];
        let service_name = service(resource);
        let bar_color = crate::theme::service_color(&service_name, self.prefs.dark);
        let name = field(span, "name");
        let toggle = self.span_collapse_key(&id);
        let has_children = self.trace_parent_ids.contains(&id);
        let label = div()
            .w(px(self.trace_label_render_width))
            .overflow_hidden()
            .flex_shrink_0()
            .flex()
            .items_center()
            .pl(px(8. + depth as f32 * 14.))
            .gap_1()
            .child(
                Button::new(SharedString::from(format!("toggle-{id}")))
                    .label(if !has_children {
                        "·"
                    } else if collapsed {
                        "▸"
                    } else {
                        "▾"
                    })
                    .disabled(!has_children)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(&toggle) {
                            this.collapsed.insert(toggle.clone());
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_ellipsis().child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(color(p.subtle))
                            .child(service_name),
                    ),
            );
        let mut timing = div()
            .relative()
            .flex_1()
            .h(px(42.))
            .child(
                div()
                    .absolute()
                    .left(relative(offset))
                    .top(px(12.))
                    .w(relative(width))
                    .h(px(18.))
                    .rounded_sm()
                    .bg(color(if error { p.error } else { bar_color })),
            )
            .child(
                div()
                    .absolute()
                    .right(px(8.))
                    .top(px(12.))
                    .text_xs()
                    .child(format_duration(dur)),
            );
        for (guide_index, fraction) in [0.25f32, 0.5, 0.75].into_iter().enumerate() {
            let time = format_duration((duration as f64 * fraction as f64) as i128);
            timing = timing.child(
                div()
                    .id(("time-guide", guide_index))
                    .absolute()
                    .left(relative(fraction))
                    .top_0()
                    .h_full()
                    .w(px(1.))
                    .bg(color(p.chrome))
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(time.clone()).build(window, cx)
                    }),
            );
        }
        let baseline = ns(&self.trace_view["traceStart"]);
        if let Some(events) = span["events"].as_array() {
            for (index, event) in events.iter().enumerate() {
                let event = event.clone();
                let event_id = id.clone();
                let position = timing_geometry(ns(&event["timestamp"]) - baseline, 0, duration).0;
                let tooltip = format!(
                    "Event · {} · {}",
                    field(&event, "name"),
                    format_timestamp(ns(&event["timestamp"]))
                );
                timing = timing.child(
                    div().absolute().left(relative(position)).top(px(0.)).child(
                        div()
                            .id(("span-event", index))
                            .test_support()
                            .role(Role::Button)
                            .aria_label(tooltip.clone())
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                    .build(window, cx)
                            })
                            .size(px(12.))
                            .ml(px(-4.))
                            .text_xs()
                            .text_color(color(p.accent))
                            .cursor_pointer()
                            .child("◆")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.select_span(event_id.clone(), cx);
                                this.data.insert("ui.trace.event".into(), event.clone());
                                cx.notify();
                            })),
                    ),
                );
            }
        }
        if let Some(logs) = self
            .data
            .get("getTraceLogSummaries")
            .and_then(Value::as_array)
        {
            for (index, log) in logs
                .iter()
                .filter(|log| log["spanID"].as_str() == Some(&id))
                .enumerate()
            {
                let log_id = field(log, "logRef");
                let position = timing_geometry(ns(&log["timestamp"]) - baseline, 0, duration).0;
                let tooltip = format!(
                    "Log · {} · {}",
                    field(log, "severityText"),
                    format_timestamp(ns(&log["timestamp"]))
                );
                timing = timing.child(
                    div()
                        .absolute()
                        .left(relative(position))
                        .top(px(28.))
                        .child(
                            div()
                                .id(("span-log-marker", index))
                                .test_support()
                                .role(Role::Button)
                                .aria_label(tooltip.clone())
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                        .build(window, cx)
                                })
                                .size(px(8.))
                                .ml(px(-4.))
                                .rounded_full()
                                .bg(color(p.accent))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.select_log(log_id.clone(), cx);
                                })),
                        ),
                )
            }
        }
        return div()
            .id(SharedString::from(format!("span-{id}")))
            .test_support()
            .w_full()
            .role(Role::TreeItem)
            .aria_selected(selected)
            .aria_expanded(!collapsed)
            .flex()
            .min_h(px(44.))
            .bg(color(if selected { p.selection } else { p.canvas }))
            .border_b_1()
            .border_color(color(p.chrome))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.trace_focus.focus(window, cx);
                this.select_span(id.clone(), cx);
            }))
            .child(label)
            .child(timing)
            .into_any_element();
    }
}

fn timing_geometry(start: i128, duration: i128, trace_duration: i128) -> (f32, f32) {
    let total = trace_duration.max(1) as f64;
    let offset = (start.max(0) as f64 / total).clamp(0., 0.998) as f32;
    let width =
        (duration.max(0) as f64 / total).clamp(0.002, (1. - offset).max(0.002) as f64) as f32;
    (offset, width)
}

#[cfg(test)]
mod tests {
    use super::timing_geometry;
    #[::core::prelude::v1::test]
    fn invalid_span_times_stay_inside_waterfall() {
        for (start, duration, total) in [
            (0, 0, 0),
            (-10, -4, 100),
            (100, 0, 100),
            (200, 50, 100),
            (0, i128::MAX, i128::MAX),
        ] {
            let (offset, width) = timing_geometry(start, duration, total);
            assert!(offset.is_finite() && width.is_finite());
            assert!((0.0..=1.0).contains(&offset));
            assert!(width > 0.0 && offset + width <= 1.00001);
        }
    }
    #[::core::prelude::v1::test]
    fn matching_spans_retain_ancestors_and_terminate_on_cycles() {
        let spans = vec![
            serde_json::json!({"matched":false,"spanData":{"spanID":"root","parentSpanID":null}}),
            serde_json::json!({"matched":true,"spanData":{"spanID":"child","parentSpanID":"root"}}),
            serde_json::json!({"matched":false,"spanData":{"spanID":"other","parentSpanID":null}}),
            serde_json::json!({"matched":true,"spanData":{"spanID":"cycle-a","parentSpanID":"cycle-b"}}),
            serde_json::json!({"matched":false,"spanData":{"spanID":"cycle-b","parentSpanID":"cycle-a"}}),
        ];
        let visible = super::matching_ancestors(&spans);
        assert!(
            visible.contains("root")
                && visible.contains("child")
                && visible.contains("cycle-a")
                && visible.contains("cycle-b")
        );
        assert!(!visible.contains("other"));
    }
    #[::core::prelude::v1::test]
    fn offsets_use_trace_baseline_not_selected_span() {
        let (offset, width) = timing_geometry(25, 50, 100);
        assert_eq!((offset, width), (0.25, 0.5));
    }
}

fn matching_ancestors(spans: &[Value]) -> std::collections::HashSet<String> {
    let parents: std::collections::HashMap<String, String> = spans
        .iter()
        .map(|n| {
            (
                field(&n["spanData"], "spanID"),
                field(&n["spanData"], "parentSpanID"),
            )
        })
        .collect();
    let mut visible = std::collections::HashSet::new();
    for node in spans
        .iter()
        .filter(|n| n["matched"].as_bool().unwrap_or(true))
    {
        let mut id = field(&node["spanData"], "spanID");
        let mut visited = std::collections::HashSet::new();
        while !id.is_empty() && visited.insert(id.clone()) {
            visible.insert(id.clone());
            let Some(parent) = parents.get(&id) else {
                break;
            };
            id = parent.clone();
        }
    }
    visible
}

pub fn register_actions(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", PreviousSpan, Some("TraceWorkspace")),
        KeyBinding::new("down", NextSpan, Some("TraceWorkspace")),
        KeyBinding::new("left", CollapseSpan, Some("TraceWorkspace")),
        KeyBinding::new("right", ExpandSpan, Some("TraceWorkspace")),
    ]);
}

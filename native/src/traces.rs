use crate::{
    app::Viewer,
    model::{field, format_duration, format_timestamp, ns, service},
    theme::{color, palette},
    widgets,
};
use gpui_kit::{
    component::{Disableable, IconName, Selectable},
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
        let available = if self.workspace_stacked(window) {
            details_width
        } else {
            f32::from(window.viewport_size().width)
                - 32.
                - if self.prefs.list_open { list_width } else { 0. }
                - if self.prefs.details_open {
                    details_width
                } else {
                    0.
                }
        };
        self.trace_label_render_width = self
            .prefs
            .trace_label_width
            .clamp(140., (available - 120.).clamp(140., 600.));
        let shell = div()
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
            .gap_4();
        let mut list_pane = None;
        let mut details_pane = None;
        if self.prefs.list_open {
            let mut list = div()
                .id("trace-list")
                .test_support()
                .w(px(list_width))
                .h_full()
                .flex()
                .flex_col()
                .bg(color(p.canvas))
                .child(
                    div()
                        .h(px(48.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .rounded_lg()
                        .bg(color(p.panel))
                        .px_4()
                        .mb_4()
                        .child(div().flex_1().child("Traces"))
                        .child(
                            widgets::button("trace-sort")
                                .icon(IconName::SortAscending)
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
                            widgets::button("trace-sort-direction")
                                .icon(if self.trace_sort % 2 == 0 {
                                    IconName::ArrowDown
                                } else {
                                    IconName::ArrowUp
                                })
                                .accessibility_label("Reverse trace sort")
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
                            .px_4()
                            .py_2()
                            .mb_2()
                            .rounded_md()
                            .bg(color(if selected { p.selection } else { p.canvas }))
                            .cursor_pointer()
                            .on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.select_trace(id.clone(), cx)
                                }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(17.))
                                            .text_ellipsis()
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .text_color(color(if ns(&row["errorCount"]) > 0 {
                                                p.error
                                            } else {
                                                p.subtle
                                            }))
                                            .child(format!(
                                                "[{} err | {} spans]",
                                                field(row, "errorCount"),
                                                field(row, "spanCount")
                                            )),
                                    ),
                            )
                            .child({
                                let parts = row["serviceBreakdown"].as_array();
                                let mut bar = div().flex().gap_1().h(px(8.)).my_2();
                                if let Some(parts) = parts.filter(|a| !a.is_empty()) {
                                    for part in parts {
                                        let count = ns(&part["count"]).max(0) as f32;
                                        if count > 0. {
                                            bar = bar.child(
                                                div()
                                                    .flex_grow(count)
                                                    .flex_basis(px(0.))
                                                    .h_full()
                                                    .rounded_full()
                                                    .bg(color(crate::theme::service_color(
                                                        &field(part, "label"),
                                                        self.prefs.dark,
                                                    ))),
                                            );
                                        }
                                    }
                                } else {
                                    bar = bar.child(div().flex_1().h_full().rounded_full().bg(
                                        color(crate::theme::service_color(
                                            &service_name,
                                            self.prefs.dark,
                                        )),
                                    ));
                                }
                                bar
                            })
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_size(px(14.))
                                    .text_color(color(p.subtle))
                                    .child(format!(
                                        "Start: {}",
                                        widgets::relative_timestamp(&row["startTime"])
                                    ))
                                    .child(format!(
                                        "Duration: {}",
                                        format_duration(ns(&row["durationNs"]))
                                    )),
                            ),
                    );
                }
            } else {
                cards = cards.child(widgets::empty("No traces in this time range", p));
            }
            list = list.child(cards);
            list_pane = Some(list.into_any_element());
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
        let title = spans
            .first()
            .map(|n| field(&n["spanData"], "name"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Select a trace".into());
        let errors = spans
            .iter()
            .filter(|n| ns(&n["spanData"]["statusCodeValue"]) == 2)
            .count();
        let mut header = div()
            .h(px(48.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_lg()
            .bg(color(p.panel))
            .child(div().flex_1().min_w_0().text_ellipsis().child(title))
            .child(
                div()
                    .text_color(color(p.subtle))
                    .child(format!("[{} spans]", spans.len())),
            )
            .child(
                widgets::button("previous-span-error")
                    .icon(IconName::ChevronLeft)
                    .accessibility_label("Previous error")
                    .tooltip("Previous error")
                    .on_click(cx.listener(|this, _, _, cx| this.navigate_span_error(false, cx))),
            )
            .child(
                div()
                    .text_color(color(p.error))
                    .child(format!("[{errors} err]")),
            )
            .child(
                widgets::button("next-span-error")
                    .icon(IconName::ChevronRight)
                    .accessibility_label("Next error")
                    .tooltip("Next error")
                    .on_click(cx.listener(|this, _, _, cx| this.navigate_span_error(true, cx))),
            )
            .child(
                widgets::button("expand-spans")
                    .icon(IconName::ChevronDown)
                    .accessibility_label("Expand all spans")
                    .tooltip("Expand all")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let prefix = format!(
                            "span:{}:",
                            this.selected_trace.as_deref().unwrap_or_default()
                        );
                        this.collapsed.retain(|k| !k.starts_with(&prefix));
                        cx.notify();
                    })),
            )
            .child(
                widgets::button("collapse-spans")
                    .icon(IconName::ChevronUp)
                    .accessibility_label("Collapse all spans")
                    .tooltip("Collapse all")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let spans = this.trace_view["spans"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        for n in spans {
                            this.collapsed
                                .insert(this.span_collapse_key(&field(&n["spanData"], "spanID")));
                        }
                        cx.notify();
                    })),
            )
            .child(
                widgets::button("trace-matches")
                    .icon(IconName::Settings2)
                    .toggled(self.trace_matches_only)
                    .selected(self.trace_matches_only)
                    .accessibility_label("Show matching spans and ancestors")
                    .tooltip("Show matches and ancestors")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.trace_matches_only = !this.trace_matches_only;
                        cx.notify();
                    })),
            );
        if let Some(trace) = self.selected_trace.clone() {
            header = header.child(
                widgets::button("export-trace")
                    .icon(IconName::ArrowDown)
                    .accessibility_label("Export trace")
                    .tooltip("Export trace")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.export_record("traces", trace.clone(), None, cx)
                    })),
            );
        }
        main = main.child(header);
        if self
            .data
            .get("ui.trace.delete")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let trace = self.selected_trace.clone().unwrap_or_default();
            main = main.child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .child("Delete this trace and all spans?")
                    .child(
                        widgets::button("confirm-trace-delete")
                            .label("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.rpc_key(
                                    "trace.delete",
                                    "deleteSpansByTraceID",
                                    json!([trace]),
                                    cx,
                                )
                            })),
                    )
                    .child(
                        widgets::button("cancel-trace-delete")
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.remove("ui.trace.delete");
                                cx.notify();
                            })),
                    ),
            );
        }
        let unplaced = ns(&self.trace_view["unplacedSpanCount"]);
        if unplaced > 0 {
            main = main.child(widgets::empty(
                format!("{unplaced} unplaced spans · inspect raw data"),
                p,
            ));
        }
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
                .mt_4()
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
        let mut footer = div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .text_color(color(p.subtle))
            .child(format!("{} spans", spans.len()));
        if let Some(trace) = self.selected_trace.clone() {
            footer = footer
                .child(
                    widgets::button("copy-trace-otlp")
                        .icon(IconName::Copy)
                        .accessibility_label("Copy trace OTLP JSON")
                        .tooltip("Copy trace OTLP JSON")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_otlp("traces", trace.clone(), None, cx)
                        })),
                )
                .child(
                    widgets::button("delete-trace")
                        .icon(IconName::Delete)
                        .accessibility_label("Delete trace")
                        .tooltip("Delete trace")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.data.insert("ui.trace.delete".into(), json!(true));
                            cx.notify();
                        })),
                );
        }
        main = main.child(footer);
        let main_pane = main.into_any_element();
        if self.prefs.details_open {
            let mut details = div()
                .id("span-details")
                .test_support()
                .w(px(details_width))
                .h_full()
                .min_h_0()
                .bg(color(p.canvas));
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
                let raw_mode = self
                    .data
                    .get("ui.trace.json")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let raw_key = format!("trace.raw.{trace_id}.{span_id}");
                let mut primary = div().flex().flex_col().min_w_0();
                primary = primary
                    .child(widgets::property("name · string", field(span, "name"), p))
                    .child(widgets::property("kind · enum", field(span, "kind"), p));
                for (key, label) in [
                    ("startTime", "startTimeUnixNano · fixed64"),
                    ("endTime", "endTimeUnixNano · fixed64"),
                ] {
                    primary = primary.child(widgets::property_exact(
                        label,
                        format!(
                            "{}\n{}",
                            format_timestamp(ns(&exact[key])),
                            field(&exact, key)
                        ),
                        field(&exact, key),
                        p,
                    ));
                }
                primary = primary.child(widgets::property(
                    "duration · int64",
                    format_duration(ns(&span["dur"])),
                    p,
                ));
                for (key, label) in [
                    ("statusCode", "status · enum"),
                    ("statusMessage", "statusMessage · string"),
                    ("spanID", "spanId · fixed64"),
                    ("parentSpanID", "parentSpanId · fixed64"),
                    ("traceState", "traceState · string"),
                    ("flags", "flags · uint32"),
                    ("droppedAttributesCount", "droppedAttributesCount · uint32"),
                    ("droppedEventsCount", "droppedEventsCount · uint32"),
                    ("droppedLinksCount", "droppedLinksCount · uint32"),
                ] {
                    primary = primary.child(widgets::property(label, field(span, key), p));
                }
                primary =
                    primary.child(widgets::property("traceId · fixed128", trace_id.clone(), p));
                for (key, label) in [
                    ("attributes", "Attributes"),
                    ("resource", "Resource"),
                    ("scope", "Scope"),
                ] {
                    primary =
                        primary
                            .child(widgets::heading(label, p))
                            .child(self.attribute_table(
                                &format!("span/{span_id}/{key}"),
                                &exact[key],
                                cx,
                            ));
                }
                let primary = if raw_mode {
                    let value = self
                        .data
                        .get(&raw_key)
                        .and_then(|v| v["content"].as_str())
                        .and_then(|text| serde_json::from_str::<Value>(text).ok());
                    if let Some(value) = value {
                        self.detail_json(&format!("span/{span_id}/otlp"), &value, 0, cx)
                    } else {
                        widgets::empty("Loading exact OTLP JSON…", p).into_any_element()
                    }
                } else {
                    primary.into_any_element()
                };
                let raw_trace = trace_id.clone();
                let raw_span = span_id.clone();
                let request_key = raw_key.clone();
                let mut actions=div().flex().items_center().gap_1().child(widgets::button("span-properties").label("Properties").toggled(!raw_mode).selected(!raw_mode).on_click(cx.listener(|this,_,_,cx|{this.data.insert("ui.trace.json".into(),json!(false));cx.notify();}))).child(widgets::button("span-json").label("JSON").toggled(raw_mode).selected(raw_mode).on_click(cx.listener(move|this,_,_,cx|{this.data.insert("ui.trace.json".into(),json!(true));this.rpc_key(&request_key,"native.export",json!({"signal":"traces","reference":raw_trace,"scope":{"spanId":raw_span}}),cx);}))).child(widgets::button("copy-span").icon(IconName::Copy).accessibility_label("Copy span JSON").tooltip("Copy span JSON").on_click(move|_,_,cx|cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))));
                let otlp_trace = trace_id.clone();
                let otlp_span = span_id.clone();
                actions = actions.child(
                    widgets::button("copy-span-otlp")
                        .icon(IconName::FileText)
                        .accessibility_label("Copy span OTLP JSON")
                        .tooltip("Copy span OTLP JSON")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_otlp(
                                "traces",
                                otlp_trace.clone(),
                                Some(json!({"spanId":otlp_span})),
                                cx,
                            )
                        })),
                );
                actions = actions.child(
                    widgets::button("export-span")
                        .icon(IconName::ArrowDown)
                        .accessibility_label("Export span")
                        .tooltip("Export span")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_record(
                                "traces",
                                trace_id.clone(),
                                Some(json!({"spanId":span_id})),
                                cx,
                            )
                        })),
                );
                let logs = self
                    .data
                    .get("span.logs")
                    .and_then(|v| v.get("logs").or(Some(v)))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let events = span["events"].as_array().cloned().unwrap_or_default();
                let activity_count = logs.len() + events.len();
                let mut activity = div().flex().flex_col().gap_2();
                for (index, event) in events.iter().enumerate() {
                    let selected_event = event.clone();
                    activity = activity.child(
                        div()
                            .px_4()
                            .py_2()
                            .child(
                                widgets::button(("span-event-detail", index))
                                    .label(field(event, "name"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.data.insert(
                                            "ui.trace.event".into(),
                                            selected_event.clone(),
                                        );
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_color(color(p.subtle))
                                    .child(format_timestamp(ns(&event["timestamp"]))),
                            ),
                    );
                }
                for (index, log) in logs.iter().enumerate() {
                    let id = field(log, "logRef");
                    let body = crate::model::attribute(&log["body"]);
                    activity = activity.child(
                        div()
                            .px_4()
                            .py_2()
                            .child(widgets::button(("span-log", index)).label(body).on_click(
                                cx.listener(move |this, _, _, cx| this.select_log(id.clone(), cx)),
                            ))
                            .child(div().text_color(color(p.subtle)).child(format!(
                                "· Log · Recorded: {}",
                                widgets::relative_timestamp(&log["timestamp"])
                            ))),
                    );
                }
                if let Some(event) = self.data.get("ui.trace.event").cloned() {
                    activity =
                        activity.child(self.attribute_table("span/selected-event", &event, cx));
                }
                let links = span["links"].as_array().cloned().unwrap_or_default();
                let link_count = links.len();
                let mut links_view = div().flex().flex_col();
                for (index, link) in links.iter().enumerate() {
                    links_view = links_view.child(self.attribute_table(
                        &format!("span/link/{index}"),
                        link,
                        cx,
                    ));
                    if let Some(trace) = link["traceID"].as_str().filter(|s| !s.is_empty()) {
                        let trace = trace.to_owned();
                        links_view = links_view.child(
                            widgets::button(("span-linked-trace", index))
                                .label("Open linked trace")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_trace(trace.clone(), cx)
                                })),
                        );
                    }
                }
                let sections = vec![
                    crate::detail_stack::DetailSection::new(
                        "summary",
                        if field(span, "parentSpanID").is_empty() {
                            "Span · root"
                        } else {
                            "Span"
                        },
                        primary,
                    )
                    .actions(actions),
                    crate::detail_stack::DetailSection::new(
                        "events",
                        format!("Events [{activity_count}]"),
                        activity,
                    )
                    .initially_open(true)
                    .available(activity_count > 0),
                    crate::detail_stack::DetailSection::new(
                        "links",
                        format!("Links [{link_count}]"),
                        links_view,
                    )
                    .initially_open(false)
                    .available(link_count > 0),
                ];
                details = details.child(self.detail_stack("traces", sections, cx));
            } else {
                details = details.child(widgets::empty("Select a span", p));
            }
            details_pane = Some(details.into_any_element());
        }
        shell
            .child(self.render_workspace(list_pane, main_pane, details_pane, window))
            .into_any_element()
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
                crate::widgets::button(SharedString::from(format!("toggle-{id}")))
                    .w(px(20.))
                    .h(px(20.))
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
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_ellipsis().child(name))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_shrink_0()
                            .text_ellipsis()
                            .text_color(color(p.subtle))
                            .child(service_name),
                    ),
            );
        let mut timing = div()
            .relative()
            .flex_1()
            .h(px(40.))
            .child(
                div()
                    .absolute()
                    .left(relative(offset))
                    .top(px(12.))
                    .w(relative(width))
                    .h(px(14.))
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
            .min_h(px(40.))
            .bg(color(if selected { p.selection } else { p.canvas }))
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

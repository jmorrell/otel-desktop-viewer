use crate::{
    app::Viewer,
    model::{field, format_timestamp, ns},
    theme::{color, palette},
    widgets,
};
use gpui_kit::{component::button::Button, *};
use serde_json::{Value, json};
use std::{sync::mpsc, time::Duration};

fn exact_value(value: &Value) -> String {
    match value {
        Value::Null => "Not recorded".into(),
        Value::String(s) if s.starts_with("0x") => u64::from_str_radix(&s[2..], 16)
            .ok()
            .map(f64::from_bits)
            .map(|n| {
                if n == 0. && n.is_sign_negative() {
                    "-0".into()
                } else {
                    n.to_string()
                }
            })
            .unwrap_or_else(|| s.clone()),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
fn point_label(point: &Value) -> String {
    let value = if !point["intValue"].is_null() {
        exact_value(&point["intValue"])
    } else if !point["doubleValue"].is_null() {
        exact_value(&point["doubleValue"])
    } else if !point["count"].is_null() {
        format!("{} observations", exact_value(&point["count"]))
    } else {
        "No value".into()
    };
    format!("{} · {value}", format_timestamp(ns(&point["timestamp"])))
}
impl Viewer {
    /// Expand an exact OTLP export on demand. Each scope gets its own request
    /// identity, so a late response cannot populate a different selection.
    pub fn raw_otlp_inspector(
        &mut self,
        instance: &str,
        signal: &str,
        reference: String,
        scope: Option<Value>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let identity = format!(
            "ui.raw.{signal}.{reference}.{}",
            scope.as_ref().map(Value::to_string).unwrap_or_default()
        );
        let open = self.data.get("ui.raw.open").and_then(Value::as_str) == Some(identity.as_str());
        let key = identity.clone();
        let signal = signal.to_owned();
        let instance = instance.to_owned();
        let mut section = div().flex().flex_col().child(
            Button::new(SharedString::from(format!("raw-toggle-{instance}")))
                .label(if open {
                    "Hide raw OTLP JSON"
                } else {
                    "View raw OTLP JSON"
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.data.get("ui.raw.open").and_then(Value::as_str) == Some(key.as_str()) {
                        this.data.remove("ui.raw.open");
                        this.data.remove(&key);
                        *this.generations.entry(key.clone()).or_default() += 1;
                        cx.notify();
                        return;
                    }
                    if let Some(old) = this
                        .data
                        .get("ui.raw.open")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    {
                        this.data.remove(&old);
                        *this.generations.entry(old).or_default() += 1;
                    }
                    this.data.insert("ui.raw.open".into(), json!(key));
                    this.data.remove(&key);
                    let Some(backend) = this.backend.clone() else {
                        cx.notify();
                        return;
                    };
                    let generation = this.generations.entry(key.clone()).or_default();
                    *generation += 1;
                    let generation = *generation;
                    let params = json!({"signal":signal,"reference":reference,"scope":scope});
                    let request_key = key.clone();
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(backend.call("native.export", params));
                    });
                    cx.spawn(async move |view, cx| {
                        loop {
                            match rx.try_recv() {
                                Ok(result) => {
                                    let _ = view.update(cx, |this, cx| {
                                        if this.generations.get(&request_key) != Some(&generation) {
                                            return;
                                        }
                                        let state = match result {
                                            Ok(value) => json!({"content":value["content"]}),
                                            Err(error) => json!({"error":format!("{error:#}")}),
                                        };
                                        this.data.insert(request_key, state);
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
                    cx.notify();
                })),
        );
        if open {
            match self.data.get(&identity).cloned() {
                Some(state) if state["error"].is_string() => {
                    section = section.child(widgets::property(
                        "Raw OTLP error",
                        field(&state, "error"),
                        p,
                    ))
                }
                Some(state) if state["content"].is_string() => {
                    let content = state["content"].as_str().unwrap_or_default();
                    match serde_json::from_str::<Value>(content) {
                        Ok(value) => {
                            section = section.child(self.detail_json(
                                &format!("{identity}.json"),
                                &value,
                                0,
                                cx,
                            ))
                        }
                        Err(_) => {
                            section = section.child(widgets::property(
                                "Raw OTLP JSON",
                                content.to_owned(),
                                p,
                            ))
                        }
                    }
                }
                _ => section = section.child(widgets::empty("Loading raw OTLP JSON…", p)),
            }
        }
        section.into_any_element()
    }
    pub fn copy_otlp(
        &mut self,
        signal: &str,
        reference: String,
        scope: Option<Value>,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let params = json!({"signal":signal, "reference":reference, "scope":scope});
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(backend.call("native.export", params));
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        let _ = view.update(cx, |this, cx| {
                            match result {
                                Ok(value) => {
                                    if let Some(content) = value["content"].as_str() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            content.to_owned(),
                                        ));
                                        this.status = "Copied OTLP JSON".into();
                                    } else {
                                        this.status =
                                            "Copy failed: export contained no JSON".into();
                                    }
                                }
                                Err(error) => this.status = format!("Copy failed: {error:#}"),
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
    /// Fetch received datapoints for the selected series, not chart-reduced samples.
    pub fn select_metric_series(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_series = Some(id.clone());
        self.selected_point = None;
        self.data.remove("metrics.bucket");
        self.data.remove("metrics.series");
        self.data.insert("metrics.points.offset".into(), json!(0));
        self.data.remove("metrics.points.last");
        if let Some(metric) = self.selected_metric.clone() {
            self.rpc_key("metrics.series","getMetricSeries",json!({"metricRef":metric,"seriesRef":id,"startTime":self.time_range.start,"endTime":self.time_range.end}),cx);
        }
        cx.notify();
    }
    pub fn render_metric_details(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let (_, width) = self.pane_widths(window);
        let metric = self
            .data
            .get("metrics.exact")
            .cloned()
            .unwrap_or(Value::Null);
        let received = self
            .data
            .get("metrics.series")
            .cloned()
            .unwrap_or(Value::Null);
        let series = if received["seriesRef"].as_str() == self.selected_series.as_deref() {
            received
        } else {
            Value::Null
        };
        let mut details = div()
            .id("metric-details")
            .w(px(width))
            .h_full()
            .overflow_y_scroll()
            .bg(color(p.panel))
            .rounded_lg()
            .child(widgets::heading("Metric details", p));
        if metric.is_null() {
            return details
                .child(widgets::empty(
                    "Select a metric to inspect its received data",
                    p,
                ))
                .into_any_element();
        }
        let metric_ref = self.selected_metric.clone().unwrap_or_default();
        let copy = serde_json::to_string_pretty(&metric).unwrap_or_default();
        let export_ref = metric_ref.clone();
        let definition_ref = metric_ref.clone();
        let copy_otlp_ref = metric_ref.clone();
        details = details.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .p_2()
                .child(
                    Button::new("copy-metric-json")
                        .label("Copy metric JSON")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                )
                .child(
                    Button::new("copy-metric-otlp")
                        .label("Copy OTLP JSON")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_otlp("metrics", copy_otlp_ref.clone(), None, cx)
                        })),
                )
                .child(
                    Button::new("export-metric")
                        .label("Export metric")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_record("metrics", export_ref.clone(), None, cx)
                        })),
                )
                .child(
                    Button::new("export-metric-definition")
                        .label("Export definition only")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_metric_definition(definition_ref.clone(), cx)
                        })),
                ),
        );
        let raw_metric = self.raw_otlp_inspector("metric", "metrics", metric_ref.clone(), None, cx);
        let mut summary = div().flex().flex_col();
        for (key, label) in [
            ("name", "Name"),
            ("description", "Description"),
            ("metricType", "Type"),
            ("unit", "Unit"),
            ("aggregationTemporalityCode", "Temporality code"),
            ("isMonotonic", "Monotonic"),
        ] {
            if !metric[key].is_null() {
                summary = summary.child(widgets::property(label, exact_value(&metric[key]), p));
            }
        }
        if matches!(
            metric["metricType"].as_str(),
            Some("Sum" | "Histogram" | "ExponentialHistogram")
        ) && !matches!(metric["aggregationTemporalityCode"].as_i64(), Some(1 | 2))
        {
            summary = summary.child(widgets::property("Temporality", "Unspecified or unknown; derived rates and histogram aggregation may be unavailable", p));
        }
        if let Some(bucket) = self.data.get("metrics.bucket").cloned() {
            if !bucket.is_null() {
                summary = summary
                    .child(widgets::heading("Selected chart bucket (derived)", p))
                    .child(
                        Button::new("clear-metric-bucket")
                            .label("Clear bucket selection")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.remove("metrics.bucket");
                                cx.notify();
                            })),
                    )
                    .child(self.detail_json("metric-derived-bucket", &bucket, 0, cx));
            }
        }
        let mut sections = vec![crate::detail_stack::DetailSection::new(
            "summary", "Metric", summary,
        )];
        for (key, label) in [
            ("metadata", "Metadata"),
            ("resource", "Resource"),
            ("scope", "Scope"),
        ] {
            let content = self.detail_json(&format!("metric-exact-{key}"), &metric[key], 0, cx);
            sections.push(crate::detail_stack::DetailSection::new(key, label, content));
        }
        let mut series_view = div().flex().flex_col();
        if let Some(items) = metric["series"].as_array() {
            series_view =
                series_view.child(widgets::heading(format!("Series · {}", items.len()), p));
            for (i, item) in items.iter().enumerate() {
                let id = field(item, "seriesRef");
                let title = item["attributes"]
                    .as_array()
                    .map(|attrs| {
                        attrs
                            .iter()
                            .map(|a| {
                                format!(
                                    "{}={}",
                                    field(a, "key"),
                                    crate::model::attribute(&a["value"])
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(" · ")
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "(no attributes)".into());
                let marker = if self.selected_series.as_ref() == Some(&id) {
                    "● "
                } else {
                    ""
                };
                series_view = series_view.child(
                    Button::new(("metric-detail-series", i))
                        .label(format!(
                            "{marker}{title} · {} points",
                            exact_value(&item["datapointCount"])
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_metric_series(id.clone(), cx)
                        })),
                );
            }
        }
        let mut points_view = div().flex().flex_col();
        if !series.is_null() {
            let series_ref = field(&series, "seriesRef");
            let export_metric = metric_ref.clone();
            let export_series = series_ref.clone();
            let copy_metric = metric_ref.clone();
            let copy_series = series_ref.clone();
            let copy = serde_json::to_string_pretty(&series).unwrap_or_default();
            series_view = series_view
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .p_2()
                        .child(
                            Button::new("copy-series-json")
                                .label("Copy series JSON")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        )
                        .child(
                            Button::new("copy-series-otlp")
                                .label("Copy series OTLP")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy_otlp(
                                        "metrics",
                                        copy_metric.clone(),
                                        Some(json!({"seriesRef":copy_series})),
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            Button::new("export-series")
                                .label("Export series")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.export_record(
                                        "metrics",
                                        export_metric.clone(),
                                        Some(json!({"seriesRef":export_series})),
                                        cx,
                                    )
                                })),
                        ),
                )
                .child(self.raw_otlp_inspector(
                    "series",
                    "metrics",
                    metric_ref.clone(),
                    Some(json!({"seriesRef":series_ref})),
                    cx,
                ))
                .child(widgets::heading("Series attributes", p))
                .child(self.detail_json("metric-series-attributes", &series["attributes"], 0, cx));
            if let Some(points) = series["datapoints"].as_array() {
                points_view = points_view.child(widgets::heading(
                    format!("Received datapoints · {}", points.len()),
                    p,
                ));
                let size = self
                    .data
                    .get("metrics.points.size")
                    .and_then(Value::as_u64)
                    .unwrap_or(25)
                    .clamp(1, 100) as usize;
                let mut offset = self
                    .data
                    .get("metrics.points.offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let selected = self.selected_point.clone().unwrap_or_default();
                let last = self
                    .data
                    .get("metrics.points.last")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if selected != last {
                    if let Some(index) = points
                        .iter()
                        .position(|point| point["datapointRef"].as_str() == Some(&selected))
                    {
                        offset = index / size * size;
                        let mut expanded = false;
                        if let Some(section) =
                            self.prefs
                                .detail_layout
                                .get_mut("metrics")
                                .and_then(|sections| {
                                    sections
                                        .iter_mut()
                                        .find(|section| section.id == "datapoints")
                                })
                        {
                            if section.collapsed {
                                section.collapsed = false;
                                expanded = true;
                            }
                        }
                        if expanded {
                            self.save_preferences();
                        }
                    }
                    self.data
                        .insert("metrics.points.last".into(), json!(selected));
                }
                if offset >= points.len() {
                    offset = points.len().saturating_sub(1) / size * size;
                }
                self.data
                    .insert("metrics.points.offset".into(), json!(offset));
                let total = points.len();
                points_view = points_view.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .p_2()
                        .child(
                            Button::new("metric-points-prev")
                                .label("Previous page")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.data.insert(
                                        "metrics.points.offset".into(),
                                        json!(offset.saturating_sub(size)),
                                    );
                                    cx.notify();
                                })),
                        )
                        .child(format!(
                            "{}–{} / {total}",
                            if total == 0 { 0 } else { offset + 1 },
                            (offset + size).min(total)
                        ))
                        .child(
                            Button::new("metric-points-next")
                                .label("Next page")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if offset + size < total {
                                        this.data.insert(
                                            "metrics.points.offset".into(),
                                            json!(offset + size),
                                        );
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Button::new("metric-points-size")
                                .label(format!("{size} / page"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let next = match size {
                                        25 => 50,
                                        50 => 100,
                                        _ => 25,
                                    };
                                    this.data.insert("metrics.points.size".into(), json!(next));
                                    this.data.insert("metrics.points.offset".into(), json!(0));
                                    cx.notify();
                                })),
                        ),
                );
                for (i, point) in points.iter().enumerate().skip(offset).take(size) {
                    let point_ref = field(point, "datapointRef");
                    let selected = self.selected_point.as_ref() == Some(&point_ref);
                    let selected_ref = point_ref.clone();
                    points_view = points_view.child(
                        Button::new(("metric-detail-point", i))
                            .label(format!(
                                "{}{}",
                                if selected { "● " } else { "" },
                                point_label(point)
                            ))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected_point = Some(selected_ref.clone());
                                this.data.remove("metrics.bucket");
                                cx.notify();
                            })),
                    );
                    if selected {
                        let copy = serde_json::to_string_pretty(point).unwrap_or_default();
                        let export_metric = metric_ref.clone();
                        let export_point = point_ref.clone();
                        let copy_metric = metric_ref.clone();
                        let copy_point = point_ref.clone();
                        points_view = points_view.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .p_2()
                                .child(
                                    Button::new(("copy-point", i))
                                        .label("Copy datapoint JSON")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                )
                                .child(
                                    Button::new(("copy-point-otlp", i))
                                        .label("Copy datapoint OTLP")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.copy_otlp(
                                                "metrics",
                                                copy_metric.clone(),
                                                Some(json!({"datapointRef":copy_point})),
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new(("export-point", i))
                                        .label("Export datapoint")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.export_record(
                                                "metrics",
                                                export_metric.clone(),
                                                Some(json!({"datapointRef":export_point})),
                                                cx,
                                            )
                                        })),
                                ),
                        );
                        points_view = points_view.child(self.raw_otlp_inspector(
                            &format!("point-{i}"),
                            "metrics",
                            metric_ref.clone(),
                            Some(json!({"datapointRef":point_ref})),
                            cx,
                        ));
                        for (key, label) in
                            [("timestamp", "Timestamp"), ("startTime", "Start time")]
                        {
                            if !point[key].is_null() {
                                points_view = points_view.child(widgets::property(
                                    label,
                                    format_timestamp(ns(&point[key])),
                                    p,
                                ));
                            }
                        }
                        for (key, label) in [
                            ("valueType", "Value type"),
                            ("intValue", "Integer value"),
                            ("doubleValue", "Double value"),
                            ("count", "Count"),
                            ("sum", "Sum"),
                            ("min", "Minimum"),
                            ("max", "Maximum"),
                            ("scale", "Scale"),
                            ("zeroCount", "Zero count"),
                            ("zeroThreshold", "Zero threshold"),
                            ("flags", "Flags"),
                        ] {
                            let optional_histogram_stat =
                                ["sum", "min", "max"].contains(&key) && !point["count"].is_null();
                            if !point[key].is_null() || optional_histogram_stat {
                                points_view = points_view.child(widgets::property(
                                    label,
                                    exact_value(&point[key]),
                                    p,
                                ));
                            }
                        }
                        points_view = points_view
                            .child(widgets::heading("Received datapoint", p))
                            .child(self.detail_json(&format!("metric-point-{i}"), point, 0, cx));
                        if let Some(exemplars) = point["exemplars"].as_array() {
                            for (j, exemplar) in exemplars.iter().enumerate() {
                                points_view = points_view
                                    .child(widgets::heading(format!("Exemplar {}", j + 1), p))
                                    .child(self.detail_json(
                                        &format!("metric-exemplar-{i}-{j}"),
                                        exemplar,
                                        0,
                                        cx,
                                    ));
                                if let Some(trace) = exemplar["traceID"]
                                    .as_str()
                                    .filter(|s| !s.is_empty() && s.chars().any(|c| c != '0'))
                                {
                                    let trace = trace.to_owned();
                                    let span = exemplar["spanID"]
                                        .as_str()
                                        .filter(|s| !s.is_empty() && s.chars().any(|c| c != '0'))
                                        .map(str::to_owned);
                                    points_view =
                                        points_view.child(
                                            Button::new((
                                                SharedString::from(format!("exemplar-trace-{i}")),
                                                j,
                                            ))
                                            .label("Open exemplar trace")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.select_trace(trace.clone(), cx);
                                                if let Some(span) = &span {
                                                    this.select_span(span.clone(), cx);
                                                }
                                            })),
                                        );
                                }
                            }
                        }
                    }
                }
            }
        }
        sections.push(crate::detail_stack::DetailSection::new(
            "series",
            "Series",
            series_view,
        ));
        sections.push(crate::detail_stack::DetailSection::new(
            "datapoints",
            "Datapoints",
            points_view,
        ));
        sections.push(crate::detail_stack::DetailSection::new(
            "otlp",
            "Raw OTLP JSON",
            raw_metric,
        ));
        details = details.child(self.detail_stack("metrics", sections, cx));
        details.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{exact_value, point_label};
    use serde_json::json;
    #[test]
    fn exact_values_keep_large_integers_and_negative_zero() {
        assert_eq!(
            exact_value(&json!("9223372036854775807")),
            "9223372036854775807"
        );
        assert_eq!(exact_value(&json!("0x8000000000000000")), "-0");
        assert!(
            point_label(&json!({"timestamp":"0","intValue":"9223372036854775807"}))
                .contains("9223372036854775807")
        );
    }
}

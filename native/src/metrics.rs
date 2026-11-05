use crate::{
    app::{Page, Viewer},
    metric_chart::{ChartData, histogram_bins},
    model::*,
    theme::{color, palette, service_color},
    widgets,
};
use gpui_kit::{component::button::Button, prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MetricPreferences {
    pub sort: usize,
    pub aggregate: bool,
    pub quantiles: bool,
    pub hidden_quantiles: HashSet<String>,
    pub heatmap: bool,
    pub mode: usize,
    pub colors: std::collections::HashMap<String, u32>,
    pub selections: std::collections::HashMap<String, Vec<String>>,
    pub stats: bool,
}
impl Default for MetricPreferences {
    fn default() -> Self {
        Self {
            sort: 0,
            aggregate: true,
            quantiles: true,
            hidden_quantiles: Default::default(),
            heatmap: true,
            mode: 0,
            colors: Default::default(),
            selections: Default::default(),
            stats: false,
        }
    }
}
pub struct MetricsState {
    pub prefs: MetricPreferences,
    pub visible: HashSet<String>,
    pub initialized_ref: String,
    pub focus: Option<FocusHandle>,
    pub bounds: Arc<Mutex<Option<Bounds<Pixels>>>>,
    pub hover: Option<(f32, f32)>,
    pub cursor: usize,
    pub confirm_delete: bool,
    pub confirm_clear: bool,
    pub color_target: Option<String>,
    pub row_focus: std::collections::HashMap<String, FocusHandle>,
}
impl Default for MetricsState {
    fn default() -> Self {
        Self {
            prefs: Default::default(),
            visible: Default::default(),
            initialized_ref: String::new(),
            focus: None,
            bounds: Default::default(),
            hover: None,
            cursor: 0,
            confirm_delete: false,
            confirm_clear: false,
            color_target: None,
            row_focus: Default::default(),
        }
    }
}
impl Viewer {
    fn save_metric_preferences(&self) {
        if !self.persist_preferences {
            return;
        }
        if let Some(dir) = directories::ProjectDirs::from("dev", "OpenTelemetry", "Desktop Viewer")
        {
            let _ = std::fs::create_dir_all(dir.config_dir());
            if let Ok(value) = serde_json::to_vec(&self.metrics.prefs) {
                let _ = std::fs::write(dir.config_dir().join("metrics.json"), value);
            }
        }
    }
    pub fn refresh_metrics(&mut self, cx: &mut Context<Self>) {
        let mut params = self.current_params(cx);
        let fields = [
            "name",
            "type",
            "serviceName",
            "description",
            "dataPointCount",
            "seriesCount",
            "lastSeen",
        ];
        params["sort"] = json!({"field":fields[(self.metrics.prefs.sort/2)%fields.len()],"direction":if self.metrics.prefs.sort%2==0{"asc"}else{"desc"}});
        self.rpc("searchMetricSummaries", params, cx);
        if let Some(id) = self.selected_metric.clone() {
            self.request_metric_view(&id, cx);
            self.rpc_key("metrics.exact", "getMetric", json!({"metricRef":id}), cx);
            if let Some(series) = self.selected_series.clone() {
                self.rpc_key("metrics.series","getMetricSeries",json!({"metricRef":id,"seriesRef":series,"startTime":self.time_range.start,"endTime":self.time_range.end}),cx);
            }
        }
    }
    pub fn load_metric(&mut self, id: String, cx: &mut Context<Self>) {
        let changed = self.selected_metric.as_ref() != Some(&id);
        self.selected_metric = Some(id.clone());
        self.page = Page::Metrics;
        if changed {
            self.metrics.confirm_delete = false;
            self.selected_series = None;
            self.selected_point = None;
            self.metrics.visible.clear();
            self.metrics.initialized_ref.clear();
            self.metrics.hover = None;
            self.data.remove("metrics.view");
            self.data.remove("metrics.exact");
            self.data.remove("metrics.series");
            self.data.remove("metrics.aggregate");
            self.data.remove("metrics.histogram-summary");
            self.data.remove("metrics.bucket");
        }
        self.request_metric_view(&id, cx);
        self.rpc_key("metrics.exact", "getMetric", json!({"metricRef":id}), cx);
        cx.notify();
    }
    pub fn request_metric_view(&mut self, id: &str, cx: &mut Context<Self>) {
        self.data.remove("metrics.contribution.request");
        self.data.remove("metrics.contribution");
        *self
            .generations
            .entry("metrics.contribution".into())
            .or_default() += 1;
        let mut selected: Vec<_> = self.metrics.visible.iter().cloned().collect();
        selected.sort();
        let initialized = self.metrics.initialized_ref == id;
        let (timezone_offset, timezone_name) = timezone_rpc_params();
        let params = json!({"metricRef":id,"startTime":self.time_range.start,"endTime":self.time_range.end,
            "targetBuckets":160,"quantiles":[0.5,0.9,0.95,0.99],"tzOffsetNs":timezone_offset,"tzName":timezone_name,
            "viewBuckets":160,"sparklineBuckets":50,"selectedSeriesRefs":selected,
            "datapointSeriesRefs":if initialized{json!(selected)}else{Value::Null},"datapointSeriesLimit":if initialized{Value::Null}else{json!(16)}});
        self.rpc_key("metrics.view", "getMetricView", params, cx);
        if initialized {
            self.metrics
                .prefs
                .selections
                .insert(id.to_owned(), selected.clone());
            self.save_metric_preferences();
            let params = json!({"metricRef":id,"startTime":self.time_range.start,"endTime":self.time_range.end,
                "targetBuckets":160,"seriesRefs":selected,"quantiles":[0.5,0.9,0.95,0.99],
                "tzOffsetNs":timezone_offset,"tzName":timezone_name,
                "viewBuckets":160,"selectedSeriesRefs":selected});
            self.rpc_key(
                "metrics.aggregate",
                "getMetricAggregateView",
                params.clone(),
                cx,
            );
            let mut summary = params;
            summary["targetBuckets"] = json!(1);
            summary["viewBuckets"] = json!(0);
            self.rpc_key(
                "metrics.histogram-summary",
                "getMetricAggregateView",
                summary,
                cx,
            );
        }
    }
    fn metric_color(&self, id: &str) -> u32 {
        self.metrics
            .prefs
            .colors
            .get(id)
            .copied()
            .unwrap_or_else(|| service_color(id, self.prefs.dark))
    }
    pub fn render_metrics(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let (list_width, details_width) = self.pane_widths(window);
        if self.metrics.focus.is_none() {
            self.metrics.focus = Some(cx.focus_handle());
            if self.persist_preferences {
                if let Some(dir) =
                    directories::ProjectDirs::from("dev", "OpenTelemetry", "Desktop Viewer")
                {
                    if let Ok(bytes) = std::fs::read(dir.config_dir().join("metrics.json")) {
                        if let Ok(prefs) = serde_json::from_slice(&bytes) {
                            self.metrics.prefs = prefs;
                        }
                    }
                }
            }
        }
        let mut shell = div().flex().size_full().min_w_0().overflow_hidden().gap_2();
        if self.prefs.list_open {
            let mut list = div()
                .id("metric-list")
                .w(px(list_width))
                .flex_shrink_0()
                .min_w_0()
                .overflow_hidden()
                .h_full()
                .flex()
                .flex_col()
                .bg(color(p.panel))
                .rounded_lg();
            let sort_label = [
                "Name",
                "Type",
                "Service",
                "Description",
                "Datapoints",
                "Series",
                "Last seen",
            ][(self.metrics.prefs.sort / 2) % 7];
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(widgets::heading("Metrics", p))
                    .child(
                        Button::new("metric-sort")
                            .label(format!(
                                "{sort_label} {}",
                                if self.metrics.prefs.sort % 2 == 0 {
                                    "↑"
                                } else {
                                    "↓"
                                }
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.metrics.prefs.sort = (this.metrics.prefs.sort + 1) % 14;
                                this.save_metric_preferences();
                                this.refresh_metrics(cx);
                            })),
                    ),
            );
            let summaries = self
                .data
                .get("searchMetricSummaries")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut cards = div().id("metric-cards").flex_1().overflow_y_scroll();
            for (i, metric) in summaries
                .iter()
                .skip(self.metrics_offset)
                .take(self.page_size)
                .enumerate()
            {
                let id = field(metric, "metricRef");
                let selected = self.selected_metric.as_ref() == Some(&id);
                let key_id = id.clone();
                let focus = self
                    .metrics
                    .row_focus
                    .entry(format!("metric:{id}"))
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                cards = cards.child(
                    div()
                        .id(("metric", i))
                        .test_support()
                        .role(Role::Button)
                        .aria_label(field(metric, "name"))
                        .aria_selected(selected)
                        .track_focus(&focus)
                        .tab_stop(true)
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                            if ["enter", "space"].contains(&event.keystroke.key.as_str()) {
                                this.select_metric(key_id.clone(), cx);
                                cx.stop_propagation();
                            }
                        }))
                        .min_w_0()
                        .overflow_hidden()
                        .p_3()
                        .m_2()
                        .rounded_md()
                        .bg(color(if selected { p.selection } else { p.canvas }))
                        .cursor_pointer()
                        .child(field(metric, "name"))
                        .child(div().text_xs().text_color(color(p.subtle)).child(format!(
                            "{} · {}",
                            field(metric, "metricType"),
                            field(metric, "serviceName")
                        )))
                        .child(div().text_xs().text_color(color(p.subtle)).child(format!(
                            "{} series · {} points",
                            field(metric, "seriesCount"),
                            field(metric, "dataPointCount")
                        )))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.select_metric(id.clone(), cx)),
                        ),
                );
            }
            if summaries.is_empty() {
                cards = cards.child(widgets::empty("No metrics in this time range", p));
            }
            let total = summaries.len();
            list = list.child(cards).child(
                div()
                    .flex()
                    .justify_between()
                    .p_2()
                    .child(Button::new("metrics-prev").label("‹").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.metrics_offset =
                                this.metrics_offset.saturating_sub(this.page_size);
                            cx.notify();
                        },
                    )))
                    .child(format!(
                        "{} / {total}",
                        (self.metrics_offset + 1).min(total)
                    ))
                    .child(Button::new("metrics-next").label("›").on_click(cx.listener(
                        move |this, _, _, cx| {
                            if this.metrics_offset + this.page_size < total {
                                this.metrics_offset += this.page_size;
                                cx.notify();
                            }
                        },
                    ))),
            );
            if self.metrics.confirm_clear {
                list = list.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .child(
                            Button::new("confirm-clear-metrics")
                                .label("Confirm delete all metrics")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.rpc_key("metrics.deleted", "clearMetrics", json!([]), cx);
                                    this.metrics.confirm_clear = false;
                                    this.selected_metric = None;
                                    this.data.remove("metrics.view");
                                    this.metrics_offset = 0;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("cancel-clear-metrics")
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.metrics.confirm_clear = false;
                                    cx.notify();
                                })),
                        ),
                );
            } else {
                list = list.child(Button::new("clear-metrics").label("Delete all…").on_click(
                    cx.listener(|this, _, _, cx| {
                        this.metrics.confirm_clear = true;
                        cx.notify();
                    }),
                ));
            }
            shell = shell.child(list);
        }
        let mut view = self
            .data
            .get("metrics.view")
            .cloned()
            .unwrap_or(Value::Null);
        let series = view["timeseries"].as_array().cloned().unwrap_or_default();
        let reference = field(&view, "metricRef");
        if !reference.is_empty() && self.metrics.initialized_ref != reference {
            self.metrics.initialized_ref = reference.clone();
            self.metrics.visible = self
                .metrics
                .prefs
                .selections
                .get(&reference)
                .map(|saved| {
                    saved
                        .iter()
                        .filter(|id| series.iter().any(|s| s["seriesRef"].as_str() == Some(id)))
                        .cloned()
                        .collect()
                })
                .unwrap_or_else(|| {
                    series
                        .iter()
                        .take(16)
                        .map(|s| field(s, "seriesRef"))
                        .collect()
                });
            self.request_metric_view(&reference, cx);
        }
        if let Some(aggregate) = self.data.get("metrics.aggregate") {
            view["aggregate"] = aggregate["aggregate"].clone();
            view["scalarAggregate"]["selected"] = aggregate["scalarAggregate"]["all"].clone();
        }
        let histogram = field(&view, "metricType").contains("Histogram");
        let mut main = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(color(p.canvas))
            .rounded_lg();
        if view.is_null() {
            main = main.child(widgets::empty(
                if self.selected_metric.is_some() {
                    "Loading metric…"
                } else {
                    "Select a metric to inspect its series and datapoints"
                },
                p,
            ));
        } else {
            main = main.child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(widgets::heading(
                                format!("{} ({})", field(&view, "name"), field(&view, "unit")),
                                p,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .when(self.metrics.confirm_delete, |row| {
                                row.child(
                                    Button::new("cancel-delete-metric")
                                        .label("Cancel")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.metrics.confirm_delete = false;
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(
                                Button::new("delete-metric")
                                    .label(if self.metrics.confirm_delete {
                                        "Confirm delete"
                                    } else {
                                        "Delete…"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.metrics.confirm_delete {
                                            if let Some(id) = this.selected_metric.clone() {
                                                this.rpc_key(
                                                    "metrics.deleted",
                                                    "deleteMetric",
                                                    json!({"metricRef":id}),
                                                    cx,
                                                );
                                                this.selected_metric = None;
                                                this.data.remove("metrics.view");
                                                this.metrics.confirm_delete = false;
                                                this.metrics_offset = 0;
                                                cx.notify();
                                            }
                                        } else {
                                            this.metrics.confirm_delete = true;
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            );
            let mut controls = div().flex().flex_wrap().gap_1().px_3();
            if histogram {
                for (heatmap, label) in [(true, "Heatmap"), (false, "Histogram")] {
                    controls = controls.child(
                        Button::new(label)
                            .label(if self.metrics.prefs.heatmap == heatmap {
                                format!("• {label}")
                            } else {
                                label.into()
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.metrics.prefs.heatmap = heatmap;
                                this.save_metric_preferences();
                                cx.notify();
                            })),
                    );
                }
                controls = controls.child(
                    Button::new("quantiles")
                        .label(if self.metrics.prefs.quantiles {
                            "Quantiles ✓"
                        } else {
                            "Quantiles"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.metrics.prefs.quantiles = !this.metrics.prefs.quantiles;
                            this.save_metric_preferences();
                            cx.notify();
                        })),
                );
            } else {
                if field(&view, "metricType") == "Gauge" {
                    self.metrics.prefs.mode = match self.metrics.prefs.mode {
                        1 => 0,
                        4 => 2,
                        other => other,
                    };
                }
                if series.len() < 2 && self.metrics.prefs.mode != 3 {
                    self.metrics.prefs.mode = 0;
                }
                let rate_available = field(&view, "metricType") == "Sum"
                    && view["aggregationTemporalityCode"].as_u64() == Some(2)
                    && view["isMonotonic"].as_bool() == Some(true);
                if self.metrics.prefs.mode == 3 && !rate_available {
                    self.metrics.prefs.mode = 0;
                }
                if rate_available {
                    for (mode, label) in [(0, "Values"), (3, "Rate")] {
                        let active = if mode == 0 {
                            self.metrics.prefs.mode != 3
                        } else {
                            self.metrics.prefs.mode == 3
                        };
                        controls = controls.child(
                            Button::new(label)
                                .label(if active {
                                    format!("• {label}")
                                } else {
                                    label.into()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.metrics.prefs.mode = mode;
                                    this.save_metric_preferences();
                                    cx.notify();
                                })),
                        );
                    }
                }
                if self.metrics.prefs.mode != 3 && series.len() > 1 {
                    for (mode, label) in [(1, "Sum"), (2, "Average")] {
                        if mode == 1 && field(&view, "metricType") != "Sum" {
                            continue;
                        }
                        let checked =
                            self.metrics.prefs.mode == mode || self.metrics.prefs.mode == 4;
                        controls = controls.child(
                            Button::new(label)
                                .label(format!("{} {label}", if checked { "☑" } else { "☐" }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.metrics.prefs.mode = match (this.metrics.prefs.mode, mode)
                                    {
                                        (4, 1) => 2,
                                        (4, 2) => 1,
                                        (1, 1) | (2, 2) => 0,
                                        (1, 2) | (2, 1) => 4,
                                        (_, m) => m,
                                    };
                                    this.save_metric_preferences();
                                    cx.notify();
                                })),
                        );
                    }
                }
                controls = controls.child(
                    Button::new("metric-stats")
                        .label(if self.metrics.prefs.stats {
                            "Stats ✓"
                        } else {
                            "Stats"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.metrics.prefs.stats = !this.metrics.prefs.stats;
                            this.save_metric_preferences();
                            cx.notify();
                        })),
                );
                controls = controls.child(
                    Button::new("aggregate")
                        .label(if self.metrics.prefs.aggregate {
                            "All series aggregate ✓"
                        } else {
                            "All series aggregate"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.metrics.prefs.aggregate = !this.metrics.prefs.aggregate;
                            this.save_metric_preferences();
                            cx.notify();
                        })),
                );
            }
            main = main.child(controls);
            if let Some(target) = self.metrics.color_target.clone() {
                let mut chooser = div().flex().flex_wrap().gap_1().px_3().py_2();
                for (index, name) in crate::theme::ink_names().iter().enumerate() {
                    let target = target.clone();
                    let shade =
                        crate::theme::ink_shades(index)[if self.prefs.dark { 0 } else { 2 }];
                    chooser = chooser.child(
                        Button::new(("metric-ink", index))
                            .label(format!("● {name}"))
                            .text_color(color(shade))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.metrics.prefs.colors.insert(target.clone(), shade);
                                this.metrics.color_target = None;
                                this.save_metric_preferences();
                                cx.notify();
                            })),
                    );
                }
                let reset = target.clone();
                chooser =
                    chooser
                        .child(
                            Button::new("metric-ink-default")
                                .label("Automatic")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.metrics.prefs.colors.remove(&reset);
                                    this.metrics.color_target = None;
                                    this.save_metric_preferences();
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("metric-ink-cancel").label("Close").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.metrics.color_target = None;
                                cx.notify();
                            }),
                        ));
                main = main.child(chooser);
            }
            let unsafe_temporality = field(&view, "metricType") != "Gauge"
                && !matches!(view["aggregationTemporalityCode"].as_u64(), Some(1 | 2));
            if unsafe_temporality {
                main = main.child(widgets::empty("Aggregation temporality is unspecified or unknown. Received datapoints remain available in Details; a chart cannot safely combine them.", p));
            } else if field(&view, "metricType") == "Summary" {
                main = main.child(widgets::empty(
                    "Received summary quantiles are available in Details.",
                    p,
                ));
            } else {
                main = main.child(self.render_metric_chart(&view, cx));
            }
            if !view["boundsMismatch"].is_null() {
                main = main.child(widgets::heading(
                    "Some histograms have incompatible bounds and cannot be combined",
                    p,
                ));
            }
            let mut rows = div()
                .id("metric-series")
                .overflow_y_scroll()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(widgets::heading(
                            format!(
                                "{} series · {} points",
                                series.len(),
                                field(&view, "datapointCount")
                            ),
                            p,
                        ))
                        .child(Button::new("toggle-series").label("All / None").on_click(
                            cx.listener(|this, _, _, cx| {
                                if this.metrics.visible.is_empty() {
                                    if let Some(series) = this
                                        .data
                                        .get("metrics.view")
                                        .and_then(|v| v["timeseries"].as_array())
                                    {
                                        this.metrics.visible =
                                            series.iter().map(|s| field(s, "seriesRef")).collect();
                                    }
                                } else {
                                    this.metrics.visible.clear();
                                }
                                if let Some(id) = this.selected_metric.clone() {
                                    this.request_metric_view(&id, cx);
                                }
                                cx.notify();
                            }),
                        )),
                );
            for (i, series) in series.iter().enumerate() {
                let id = field(series, "seriesRef");
                let selected = self.selected_series.as_ref() == Some(&id);
                let checked = self.metrics.visible.contains(&id);
                let toggle = id.clone();
                let choose = id.clone();
                let key_choose = id.clone();
                let row_focus = self
                    .metrics
                    .row_focus
                    .entry(format!("series:{id}"))
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                let tint = self.metric_color(&id);
                let label = series["attributes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|a| format!("{}={}", field(a, "key"), attribute(&a["value"])))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    })
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "(no attributes)".into());
                let color_id = id.clone();
                let mut row = div()
                    .id(("series", i))
                    .test_support()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .bg(color(if selected { p.selection } else { p.canvas }))
                    .border_b_1()
                    .border_color(color(p.chrome))
                    .child(
                        Button::new(("visible", i))
                            .label(if checked { "☑" } else { "☐" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.metrics.visible.remove(&toggle) {
                                    this.metrics.visible.insert(toggle.clone());
                                }
                                if let Some(id) = this.selected_metric.clone() {
                                    this.request_metric_view(&id, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("series-color", i))
                            .label("●")
                            .text_color(color(tint))
                            .accessibility_label("Choose series color")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.metrics.color_target = Some(color_id.clone());
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(("series-select", i))
                            .test_support()
                            .role(Role::Button)
                            .aria_label(format!("Inspect series {label}"))
                            .track_focus(&row_focus)
                            .tab_stop(true)
                            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                                if ["enter", "space"].contains(&event.keystroke.key.as_str()) {
                                    this.select_metric_series(key_choose.clone(), cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .flex_1()
                            .min_w_0()
                            .cursor_pointer()
                            .text_color(color(tint))
                            .child(
                                div()
                                    .w_full()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(label),
                            )
                            .child(div().text_xs().text_color(color(p.subtle)).child(format!(
                                "{} points · {}",
                                field(series, "datapointCount"),
                                service(&series["resource"])
                            )))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_metric_series(choose.clone(), cx)
                            })),
                    );
                let points: Vec<_> = series["sparkline"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((ns(&p["timestamp"]), number(&p["value"])?)))
                    .collect();
                if !points.is_empty() {
                    row = row.child(sparkline(points, tint).w(px(100.)).h(px(32.)));
                }
                rows = rows.child(row);
            }
            main = main.child(rows);
        }
        shell = shell.child(main);
        if self.prefs.details_open {
            shell = shell.child(
                div()
                    .w(px(details_width))
                    .flex_shrink_0()
                    .min_w_0()
                    .overflow_hidden()
                    .h_full()
                    .child(self.render_metric_details(window, cx)),
            );
        }
        shell.into_any_element()
    }

    fn render_metric_chart(&mut self, view: &Value, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let histogram = field(view, "metricType").contains("Histogram");
        // A contribution is computed by the store against the same checked pool
        // and time grid. In particular, cumulative readings must be differenced
        // before highlighting their activity; the raw received counts are not it.
        let contribution_request = if histogram
            && self
                .selected_series
                .as_ref()
                .is_some_and(|id| self.metrics.visible.contains(id))
            && self.selected_point.is_some()
        {
            let mut selected: Vec<_> = self.metrics.visible.iter().cloned().collect();
            selected.sort();
            let (offset, zone) = timezone_rpc_params();
            Some(
                json!({"metricRef":field(view,"metricRef"),"datapointRef":self.selected_point,
                "startTime":self.time_range.start,"endTime":self.time_range.end,
                "targetBuckets":if self.metrics.prefs.heatmap {160}else{1},"seriesRefs":selected,
                "quantiles":[],"tzOffsetNs":offset,"tzName":zone,"viewBuckets":0}),
            )
        } else {
            None
        };
        if self.data.get("metrics.contribution.request") != contribution_request.as_ref() {
            self.data.remove("metrics.contribution");
            if let Some(params) = contribution_request {
                self.data
                    .insert("metrics.contribution.request".into(), params.clone());
                self.rpc_key("metrics.contribution", "getMetricAggregateView", params, cx);
            } else {
                self.data.remove("metrics.contribution.request");
                *self
                    .generations
                    .entry("metrics.contribution".into())
                    .or_default() += 1;
            }
        }
        let contribution = self
            .data
            .get("metrics.contribution")
            .and_then(|v| v["contribution"].as_array())
            .and_then(|a| a.first())
            .cloned();
        let contribution_bins = contribution
            .as_ref()
            .map(histogram_bins)
            .unwrap_or_default();
        let contribution_color = self
            .selected_series
            .as_deref()
            .map(|id| self.metric_color(id))
            .unwrap_or(p.text);
        let mut data = ChartData::from_view(
            view,
            self.metrics.prefs.mode,
            &self.metrics.visible,
            self.metrics.prefs.aggregate,
            self.metrics.prefs.quantiles,
            |id| self.metric_color(id),
        );
        if histogram {
            data.curves.retain(|curve| {
                !self
                    .metrics
                    .prefs
                    .hidden_quantiles
                    .contains(curve.id.strip_prefix('p').unwrap_or(&curve.id))
            });
        }
        let overlay_series = self
            .selected_series
            .as_deref()
            .and_then(|id| {
                view["timeseries"]
                    .as_array()
                    .and_then(|s| s.iter().find(|s| s["seriesRef"].as_str() == Some(id)))
            })
            .or_else(|| {
                view["timeseries"].as_array().and_then(|s| {
                    s.iter()
                        .find(|s| self.metrics.visible.contains(&field(s, "seriesRef")))
                })
            });
        let stats: Vec<(String, f64)> = if self.metrics.prefs.stats && !histogram {
            overlay_series
                .into_iter()
                .flat_map(|series| {
                    let key = if self.metrics.prefs.mode == 3 {
                        "rateStats"
                    } else {
                        "stats"
                    };
                    ["min", "avg", "max"].into_iter().filter_map(move |name| {
                        number(&series[key][name])
                            .filter(|v| v.is_finite())
                            .map(|value| (name.to_owned(), value))
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let painted_stats = stats.clone();
        let histogram_summary = self
            .data
            .get("metrics.histogram-summary")
            .and_then(|v| v["aggregate"].as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null);
        let bins = histogram_bins(&histogram_summary);
        let histogram_bins_for_hover = bins.clone();
        let quantile_colors: Vec<_> = if histogram && self.metrics.prefs.quantiles {
            ["0.5", "0.9", "0.95", "0.99"]
                .into_iter()
                .filter(|q| !self.metrics.prefs.hidden_quantiles.contains(*q))
                .filter_map(|q| {
                    number(&histogram_summary["quantiles"][q])
                        .filter(|value| value.is_finite())
                        .map(|value| (q.to_owned(), value, self.metric_color(q)))
                })
                .collect()
        } else {
            Vec::new()
        };
        let painted_quantiles = quantile_colors.clone();
        let painted_contribution = contribution.clone();
        let painted_contribution_bins = contribution_bins.clone();
        let plot = data.clone();
        let axis = data.clone();
        let bounds = self.metrics.bounds.clone();
        let prepaint = bounds.clone();
        let heatmap = self.metrics.prefs.heatmap;
        let focus = self.metrics.focus.clone().unwrap();
        let focus_on_click = focus.clone();
        let hover = self.metrics.hover;
        let unit = field(view, "unit");
        let empty = self.metrics.visible.is_empty()
            || (data.curves.iter().all(|c| c.points.is_empty())
                && data
                    .columns
                    .iter()
                    .all(|c| c.bins.iter().all(|b| b.count == 0.)));
        let chart = canvas(
            move |b, _, _| {
                *prepaint.lock().unwrap() = Some(b);
            },
            move |b, _, window, _| {
                for n in 0..5 {
                    let y = b.origin.y + b.size.height * (n as f32 / 4.);
                    let mut line = PathBuilder::stroke(px(1.));
                    line.move_to(point(b.origin.x, y));
                    line.line_to(point(b.right(), y));
                    if let Ok(line) = line.build() {
                        window.paint_path(line, color(p.chrome));
                    }
                }
                if histogram && heatmap {
                    for (i, column) in plot.columns.iter().enumerate() {
                        let x1 = plot.x(column.start);
                        let x2 = plot.x(column.time);
                        let fallback = 1. / plot.columns.len().max(1) as f32;
                        let (x1, x2) = if x1 == x2 {
                            if plot.columns.len() == 1 {
                                (0., 1.)
                            } else {
                                ((x2 - fallback / 2.).max(0.), (x2 + fallback / 2.).min(1.))
                            }
                        } else {
                            (x1, x2)
                        };
                        let width = (x2 - x1).max(0.001);
                        for bin in &column.bins {
                            if bin.count <= 0. {
                                continue;
                            }
                            let lower = if bin.lower.is_finite() {
                                bin.lower
                            } else {
                                plot.y_min
                            };
                            let upper = if bin.upper.is_finite() {
                                bin.upper
                            } else {
                                plot.y_max
                            };
                            let y1 = plot.y(upper);
                            let y2 = plot.y(lower);
                            let alpha =
                                (bin.count.ln_1p() / plot.count_max.ln_1p()).clamp(0.08, 1.) as f32;
                            window.paint_quad(fill(
                                Bounds {
                                    origin: b.origin + point(b.size.width * x1, b.size.height * y1),
                                    size: size(
                                        (b.size.width * width).max(px(1.)),
                                        (b.size.height * (y2 - y1)).max(px(2.)),
                                    ),
                                },
                                color(p.accent).opacity(alpha),
                            ));
                        }
                        let _ = i;
                    }
                } else if histogram {
                    let maximum = bins.iter().map(|b| b.count).fold(1., f64::max);
                    for (i, bin) in bins.iter().enumerate() {
                        let width = 1. / bins.len().max(1) as f32;
                        let height = (bin.count / maximum) as f32;
                        window.paint_quad(fill(
                            Bounds {
                                origin: b.origin
                                    + point(
                                        b.size.width * (i as f32 * width),
                                        b.size.height * (1. - height),
                                    ),
                                size: size(
                                    (b.size.width * width - px(1.)).max(px(1.)),
                                    b.size.height * height,
                                ),
                            },
                            color(p.accent),
                        ));
                        if let Some(selected) = painted_contribution_bins.iter().find(|selected| {
                            selected.lower == bin.lower && selected.upper == bin.upper
                        }) {
                            let selected_height = (selected.count / maximum).clamp(0., 1.) as f32;
                            if selected_height > 0. {
                                let rect = Bounds {
                                    origin: b.origin
                                        + point(
                                            b.size.width * (i as f32 * width),
                                            b.size.height * (1. - selected_height),
                                        ),
                                    size: size(
                                        (b.size.width * width - px(1.)).max(px(1.)),
                                        b.size.height * selected_height,
                                    ),
                                };
                                window.paint_quad(fill(
                                    rect,
                                    color(contribution_color).opacity(0.65),
                                ));
                                paint_outline(rect, contribution_color, window);
                            }
                        }
                    }
                }
                if histogram && heatmap {
                    if let Some(contribution) = &painted_contribution {
                        let start = plot.x(ns(&contribution["startTime"]));
                        let end = plot.x(ns(&contribution["timestamp"]));
                        let (start, end) = if start == end && plot.columns.len() == 1 {
                            (0., 1.)
                        } else {
                            (start, end)
                        };
                        for bin in &painted_contribution_bins {
                            if bin.count <= 0. {
                                continue;
                            }
                            let top = plot.y(if bin.upper.is_finite() {
                                bin.upper
                            } else {
                                plot.y_max
                            });
                            let bottom = plot.y(if bin.lower.is_finite() {
                                bin.lower
                            } else {
                                plot.y_min
                            });
                            let rect = Bounds {
                                origin: b.origin + point(b.size.width * start, b.size.height * top),
                                size: size(
                                    (b.size.width * (end - start)).max(px(2.)),
                                    (b.size.height * (bottom - top)).max(px(2.)),
                                ),
                            };
                            paint_outline(rect, contribution_color, window);
                        }
                    }
                }
                if histogram && !heatmap {
                    for (_, value, tint) in &painted_quantiles {
                        if let Some((i, bin)) = bins
                            .iter()
                            .enumerate()
                            .find(|(_, bin)| bin.lower <= *value && *value <= bin.upper)
                        {
                            let within = if bin.lower.is_finite()
                                && bin.upper.is_finite()
                                && bin.upper > bin.lower
                            {
                                ((*value - bin.lower) / (bin.upper - bin.lower)) as f32
                            } else {
                                0.5
                            };
                            let x = b.origin.x
                                + b.size.width * ((i as f32 + within) / bins.len().max(1) as f32);
                            let mut line = PathBuilder::stroke(px(2.));
                            line.move_to(point(x, b.origin.y));
                            line.line_to(point(x, b.bottom()));
                            if let Ok(line) = line.build() {
                                window.paint_path(line, color(*tint));
                            }
                        }
                    }
                }
                if !histogram || heatmap {
                    for curve in &plot.curves {
                        let mut path = PathBuilder::stroke(px(2.));
                        for (i, point_) in curve.points.iter().enumerate() {
                            let pos = b.origin
                                + point(
                                    b.size.width * plot.x(point_.time),
                                    b.size.height * plot.y(point_.value),
                                );
                            if i == 0 {
                                path.move_to(pos);
                            } else {
                                path.line_to(pos);
                            }
                            if point_.reset {
                                let mut marker = PathBuilder::stroke(px(1.5));
                                marker.move_to(pos - point(px(4.), px(6.)));
                                marker.line_to(pos + point(px(4.), px(6.)));
                                marker.move_to(pos + point(px(4.), px(-6.)));
                                marker.line_to(pos + point(px(-4.), px(6.)));
                                if let Ok(marker) = marker.build() {
                                    window.paint_path(marker, color(p.error));
                                }
                            }
                            window.paint_quad(fill(
                                Bounds {
                                    origin: pos - point(px(2.), px(2.)),
                                    size: size(px(4.), px(4.)),
                                },
                                color(curve.color),
                            ));
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color(curve.color));
                        }
                    }
                }
                for (_, value) in &painted_stats {
                    let y = b.origin.y + b.size.height * plot.y(*value);
                    for segment in 0..40 {
                        let mut line = PathBuilder::stroke(px(1.));
                        let x = b.origin.x + b.size.width * (segment as f32 / 40.);
                        line.move_to(point(x, y));
                        line.line_to(point(x + b.size.width / 80., y));
                        if let Ok(line) = line.build() {
                            window.paint_path(line, color(p.subtle).opacity(0.6));
                        }
                    }
                }
                if let Some((x, _)) = hover {
                    let mut cursor = PathBuilder::stroke(px(1.));
                    cursor.move_to(b.origin + point(b.size.width * x, px(0.)));
                    cursor.line_to(b.origin + point(b.size.width * x, b.size.height));
                    if let Ok(path) = cursor.build() {
                        window.paint_path(path, color(p.text));
                    }
                }
            },
        )
        .size_full();
        let hover_bounds = bounds.clone();
        let click_data = data.clone();
        let keyboard_data = data.clone();
        let keyboard_bins = histogram_bins_for_hover.len();
        let mut root = div()
            .id("metric-chart")
            .test_support()
            .role(Role::Group)
            .aria_label("Metric chart. Use arrow keys to explore datapoints and buckets")
            .track_focus(&focus)
            .tab_stop(true)
            .relative()
            .m_3()
            .h(px(310.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if let Some(b) = *hover_bounds.lock().unwrap() {
                    if !b.contains(&event.position) {
                        if this.metrics.hover.take().is_some() {
                            cx.notify();
                        }
                        return;
                    }
                    let x = f32::from((event.position.x - b.origin.x) / b.size.width).clamp(0., 1.);
                    let y =
                        f32::from((event.position.y - b.origin.y) / b.size.height).clamp(0., 1.);
                    this.metrics.hover = Some((x, y));
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    if let Some(b) = *bounds.lock().unwrap() {
                        if !b.contains(&event.position) {
                            return;
                        }
                        focus_on_click.focus(window, cx);
                        let x =
                            f32::from((event.position.x - b.origin.x) / b.size.width).clamp(0., 1.);
                        let y = f32::from((event.position.y - b.origin.y) / b.size.height)
                            .clamp(0., 1.);
                        this.metrics.hover = Some((x, y));
                        this.select_chart_at(&click_data, x, cx);
                    }
                }),
            )
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let count = if histogram {
                    if heatmap {
                        keyboard_data.columns.len()
                    } else {
                        keyboard_bins
                    }
                } else {
                    keyboard_data
                        .curves
                        .iter()
                        .map(|c| c.points.len())
                        .sum::<usize>()
                };
                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.metrics.hover = None;
                        this.selected_point = None;
                        this.data.remove("metrics.bucket");
                        cx.notify();
                        return;
                    }
                    "left" => this.metrics.cursor = this.metrics.cursor.saturating_sub(1),
                    "right" => {
                        this.metrics.cursor = (this.metrics.cursor + 1).min(count.saturating_sub(1))
                    }
                    "home" => this.metrics.cursor = 0,
                    "end" => this.metrics.cursor = count.saturating_sub(1),
                    _ => return,
                }
                if histogram && !heatmap {
                    let x = (this.metrics.cursor as f32 + 0.5) / keyboard_bins.max(1) as f32;
                    this.metrics.hover = Some((x, 0.5));
                    this.select_chart_at(&keyboard_data, x, cx);
                } else if histogram {
                    if let Some(column) = keyboard_data.columns.get(this.metrics.cursor) {
                        let x = keyboard_data.x(column.time);
                        this.metrics.hover = Some((x, 0.5));
                        this.select_chart_at(&keyboard_data, x, cx);
                    }
                }
                let point = keyboard_data
                    .curves
                    .iter()
                    .flat_map(|c| c.points.iter())
                    .nth(this.metrics.cursor);
                if let Some(point) = point {
                    let x = keyboard_data.x(point.time);
                    this.metrics.hover = Some((x, keyboard_data.y(point.value)));
                    this.select_chart_at(&keyboard_data, x, cx);
                }
                cx.notify();
            }));
        root = root
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(color(p.subtle))
                    .child(if histogram && !heatmap {
                        format!(
                            "{} observations",
                            histogram_bins_for_hover
                                .iter()
                                .map(|b| b.count)
                                .fold(0., f64::max)
                        )
                    } else {
                        format!("{:.4} {unit}", data.y_max)
                    })
                    .child(if histogram {
                        "Observation distribution".to_owned()
                    } else {
                        [
                            "Values",
                            "Values + sum",
                            "Values + average",
                            "Rate",
                            "Values + sum + average",
                        ][self.metrics.prefs.mode.min(4)]
                        .to_owned()
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(chart)
                    .children(if empty {
                        Some(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(color(p.subtle))
                                .child(if self.metrics.visible.is_empty() {
                                    "Select a timeseries below to draw its data"
                                } else {
                                    "No observations to chart in this time range"
                                }),
                        )
                    } else {
                        None
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(color(p.subtle))
                    .child(if histogram && !heatmap {
                        histogram_bins_for_hover
                            .first()
                            .map(|b| format!("{} {unit}", b.lower))
                            .unwrap_or_default()
                    } else {
                        chart_time(axis.x_min)
                    })
                    .child(if histogram && !heatmap {
                        histogram_bins_for_hover
                            .last()
                            .map(|b| format!("{} {unit}", b.upper))
                            .unwrap_or_default()
                    } else {
                        chart_time(axis.x_max)
                    }),
            );
        let mut legend = div().flex().flex_wrap().gap_3().text_xs();
        for curve in data
            .curves
            .iter()
            .filter(|c| !c.points.is_empty() && !histogram)
        {
            legend = legend.child(
                div()
                    .text_color(color(curve.color))
                    .child(format!("● {}", curve.label)),
            );
        }
        if histogram {
            for q in ["0.5", "0.9", "0.95", "0.99"] {
                let enabled = !self.metrics.prefs.hidden_quantiles.contains(q);
                let label = format!(
                    "p{}{}",
                    q.parse::<f64>().unwrap() * 100.,
                    if !heatmap {
                        number(&histogram_summary["quantiles"][q])
                            .map(|v| format!(" {v:.4} {unit}"))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                );
                legend = legend.child(
                    Button::new(SharedString::from(format!("quantile-{q}")))
                        .label(label)
                        .toggled(self.metrics.prefs.quantiles && enabled)
                        .text_color(color(self.metric_color(q)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let enabled = this.metrics.prefs.quantiles
                                && !this.metrics.prefs.hidden_quantiles.contains(q);
                            this.metrics.prefs.quantiles = true;
                            if enabled {
                                this.metrics.prefs.hidden_quantiles.insert(q.into());
                            } else {
                                this.metrics.prefs.hidden_quantiles.remove(q);
                            }
                            this.save_metric_preferences();
                            cx.notify();
                        })),
                );
            }
        }
        root = root.child(legend);
        if let Some(contribution) = &contribution {
            root = root.child(div().text_xs().text_color(color(contribution_color)).child(
                format!(
                    "Selected datapoint contribution · {} interval observations",
                    field(contribution, "count")
                ),
            ));
        }
        if !stats.is_empty() {
            root = root.child(div().text_xs().text_color(color(p.subtle)).child(format!(
                    "Selected series · {}",
                    stats
                        .iter()
                        .map(|(name, value)| format!("{name} {value:.4} {unit}"))
                        .collect::<Vec<_>>()
                        .join(" · ")
                )));
        }
        if let Some((x, _)) = hover {
            let label = if histogram && !heatmap {
                let index = ((histogram_bins_for_hover.len() as f32 * x) as usize)
                    .min(histogram_bins_for_hover.len().saturating_sub(1));
                histogram_bins_for_hover
                    .get(index)
                    .map(|bin| {
                        format!(
                            "({}, {}] {unit} · {} observations in selected window",
                            bin.lower, bin.upper, bin.exact_count
                        )
                    })
                    .unwrap_or_default()
            } else if histogram {
                data.nearest_column(x)
                    .map(|c| {
                        let value = data.y_max
                            - hover.map_or(0.5, |h| h.1) as f64 * (data.y_max - data.y_min);
                        let bin = c.bins.iter().find(|b| b.lower <= value && value <= b.upper);
                        format!(
                            "{} · {} observations{}",
                            format_timestamp(c.time),
                            field(&c.value, "count"),
                            bin.map(|b| format!(
                                " · ({}, {}] {unit}: {}",
                                b.lower, b.upper, b.exact_count
                            ))
                            .unwrap_or_default()
                        )
                    })
                    .unwrap_or_default()
            } else {
                data.nearest_xy(x, hover.map_or(0.5, |h| h.1))
                    .map(|(c, p)| {
                        format!(
                            "{} · {} = {} {unit}{}{}",
                            format_timestamp(p.time),
                            c.label,
                            p.display,
                            if p.reset { " · counter reset" } else { "" },
                            p.slope
                                .map(|s| format!(" · slope {s:.6}/s"))
                                .unwrap_or_default()
                        )
                    })
                    .unwrap_or_default()
            };
            root = root.child(
                div()
                    .h(px(20.))
                    .flex_shrink_0()
                    .text_ellipsis()
                    .text_xs()
                    .text_color(color(p.text))
                    .child(label),
            );
        } else {
            // Reserve the readout row so moving from the plot to its legend
            // cannot shift a button between mouse-down and activation.
            root = root.child(div().h(px(20.)).flex_shrink_0());
        }
        root.into_any_element()
    }
    fn select_chart_at(&mut self, data: &ChartData, x: f32, cx: &mut Context<Self>) {
        if !data.columns.is_empty() {
            self.selected_point = None;
            if !self.metrics.prefs.heatmap {
                if let Some(summary) = self
                    .data
                    .get("metrics.histogram-summary")
                    .and_then(|v| v["aggregate"].as_array())
                    .and_then(|a| a.first())
                    .cloned()
                {
                    let bins = histogram_bins(&summary);
                    let i = ((bins.len() as f32 * x) as usize).min(bins.len().saturating_sub(1));
                    if let Some(bin) = bins.get(i) {
                        self.data.insert("metrics.bucket".into(), json!({"selection":"Histogram window bucket","lower":bin.lower.to_string(),"upper":bin.upper.to_string(),"count":bin.exact_count,"window":summary}));
                    }
                }
            } else if let Some(column) = data.nearest_column(x) {
                self.data
                    .insert("metrics.bucket".into(), column.value.clone());
            }
            cx.notify();
            return;
        }
        if let Some((curve, point)) = data.nearest_xy(x, self.metrics.hover.map_or(0.5, |h| h.1)) {
            let id = curve.id.clone();
            let point_id = point.source.clone();
            if !id.starts_with("aggregate:") {
                self.select_metric_series(id, cx);
                if !point_id.is_empty() {
                    self.selected_point = Some(point_id);
                    self.data.remove("metrics.bucket");
                } else {
                    self.data.insert("metrics.bucket".into(),json!({"selection":"Derived chart value","seriesRef":curve.id,"timestamp":point.time.to_string(),"value":point.value,"slope":point.slope,"hasReset":point.reset}));
                }
            } else {
                self.data.insert("metrics.bucket".into(),json!({"selection":"Cross-series aggregate","aggregate":curve.label,"timestamp":point.time.to_string(),"value":point.display,"slope":point.slope}));
            }
        }
        cx.notify();
    }
}
fn paint_outline(bounds: Bounds<Pixels>, tint: u32, window: &mut Window) {
    let mut outline = PathBuilder::stroke(px(2.));
    outline.move_to(bounds.origin);
    outline.line_to(point(bounds.right(), bounds.top()));
    outline.line_to(point(bounds.right(), bounds.bottom()));
    outline.line_to(point(bounds.left(), bounds.bottom()));
    outline.close();
    if let Ok(outline) = outline.build() {
        window.paint_path(outline, color(tint));
    }
}
fn sparkline(points: Vec<(i128, f64)>, tint: u32) -> impl Styled + IntoElement {
    canvas(
        |_, _, _| (),
        move |b, _, window, _| {
            let min_t = points.first().map_or(0, |p| p.0);
            let max_t = points.last().map_or(1, |p| p.0);
            let min = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
            let max = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
            let mut path = PathBuilder::stroke(px(1.5));
            for (i, (t, v)) in points.iter().enumerate() {
                let p = b.origin
                    + point(
                        b.size.width * ((t - min_t) as f64 / (max_t - min_t).max(1) as f64) as f32,
                        b.size.height * (1. - (v - min) / (max - min).max(1e-12)) as f32,
                    );
                if i == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color(tint));
            }
        },
    )
}

fn chart_time(time: i128) -> String {
    format_timestamp(time)
        .split_once(' ')
        .map(|(_, time)| time.to_owned())
        .unwrap_or_else(|| format_timestamp(time))
}

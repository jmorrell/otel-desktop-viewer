use crate::app::Viewer;
use gpui_kit::*;
use serde_json::json;
use std::{path::PathBuf, sync::mpsc, time::Duration};
impl Viewer {
    pub fn import_files(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let files = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import OTLP JSON or JSONL".into()),
        });
        cx.spawn(async move |view, cx| {
            if let Ok(Ok(Some(paths))) = files.await {
                let _ = view.update(cx, |this, cx| this.import_paths(paths, cx));
            }
        })
        .detach();
    }
    pub fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        if self.import_active {
            self.status = "An import is already running".into();
            cx.notify();
            return;
        }
        self.import_active = true;
        self.import_cancel
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let cancelled = self.import_cancel.clone();
        self.status = format!("Importing {} files…", paths.len());
        cx.notify();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reports = Vec::new();
            for path in paths {
                if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    reports.push(json!({"cancelled":true,"message":"Import cancelled"}));
                    break;
                }
                reports.push(match backend.import_file_cancellable(&path, &cancelled) {
                    Ok(report) => json!({"file":path.display().to_string(),"requests":report.requests,"issues":report.issues,"cancelled":report.cancelled}),
                    Err(error) => json!({"file":path.display().to_string(),"error":format!("{error:#}")}),
                });
            }
            let _ = tx.send(reports);
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(reports) => {
                        let _ = view.update(cx, |this, cx| {
                            this.import_active = false;
                            let batches: usize = reports
                                .iter()
                                .map(|r| r["requests"].as_u64().unwrap_or(0) as usize)
                                .sum();
                            this.status = format!(
                                "Import finished · {} files · {batches} batches",
                                reports.len()
                            );
                            this.data.insert("import.report".into(), json!(reports));
                            this.import_report_open = true;
                            this.refresh_search_definitions(cx);
                            this.refresh(cx);
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
    }
    pub fn export_record(
        &mut self,
        signal: &str,
        reference: String,
        scope: Option<serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        self.export_record_options(signal, reference, scope, true, cx);
    }
    pub fn export_metric_definition(&mut self, reference: String, cx: &mut Context<Self>) {
        self.export_record_options("metrics", reference, None, false, cx);
    }
    pub fn export_record_options(
        &mut self,
        signal: &str,
        reference: String,
        scope: Option<serde_json::Value>,
        include_datapoints: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let mut params = json!({"signal":signal,"reference":reference,"scope":scope});
        if signal == "metrics" {
            params["includeDatapoints"] = json!(include_datapoints);
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(backend.call("native.export", params));
        });
        cx.spawn(async move |view, cx| {
            loop {
                match rx.try_recv() {
                    Ok(result) => {
                        match result {
                            Ok(value) => {
                                let filename = value["filename"]
                                    .as_str()
                                    .unwrap_or("telemetry.json")
                                    .to_owned();
                                let content =
                                    value["content"].as_str().unwrap_or_default().to_owned();
                                let prompt = cx.update(|cx| {
                                    cx.prompt_for_new_path(
                                        &std::env::var_os("HOME")
                                            .map(PathBuf::from)
                                            .unwrap_or_else(|| "/tmp".into()),
                                        Some(&filename),
                                    )
                                });
                                if let Ok(Ok(Some(path))) = prompt.await {
                                    let result = std::fs::write(&path, content.as_bytes());
                                    let _ = view.update(cx, |this, cx| {
                                        this.status = match result {
                                            Ok(()) => format!("Exported {}", path.display()),
                                            Err(e) => format!("Export failed: {e}"),
                                        };
                                        cx.notify();
                                    });
                                }
                            }
                            Err(error) => {
                                let _ = view.update(cx, |this, cx| {
                                    this.status = format!("Export failed: {error:#}");
                                    cx.notify();
                                });
                            }
                        }
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        smol::Timer::after(Duration::from_millis(25)).await;
                    }
                    Err(_) => break,
                }
            }
        })
        .detach();
    }
    pub fn set_relative_time(&mut self, minutes: Option<i64>, cx: &mut Context<Self>) {
        self.time_range.end = None;
        self.time_range.relative_minutes = minutes;
        self.time_range.label = match minutes {
            None => "All time".into(),
            Some(m) if m % 1440 == 0 => format!("Last {}d", m / 1440),
            Some(m) if m % 60 == 0 => format!("Last {}h", m / 60),
            Some(m) => format!("Last {m}m"),
        };
        self.time_open = false;
        self.time_range.start = minutes.map(|m| {
            (chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as i128
                - m as i128 * 60_000_000_000)
                .to_string()
        });
        self.refresh(cx);
    }
}

impl Viewer {
    pub fn render_appearance(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::button::Button;
        let p = crate::theme::palette(self.prefs.dark);
        let names = crate::theme::ink_names();
        let selected = if self.prefs.ink == usize::MAX {
            names.iter().position(|s| s == "to-ro").unwrap_or(0)
        } else {
            self.prefs.ink % names.len()
        };
        let mut seasons = div().flex().gap_1();
        for (index, name) in ["Haru", "Natsu", "Aki", "Fuyu"].iter().enumerate() {
            seasons = seasons.child(Button::new(("season", index)).label(*name).on_click(
                cx.listener(move |this, _, _, cx| {
                    this.prefs.season = index;
                    this.update_appearance(cx);
                }),
            ))
        }
        let count = names.len();
        let mut inks = div()
            .id("appearance-inks")
            .test_support()
            .track_focus(&self.appearance_focus)
            .tab_stop(true)
            .role(Role::RadioGroup)
            .aria_label("Accent ink")
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let current = if this.prefs.ink == usize::MAX {
                    selected
                } else {
                    this.prefs.ink % count
                };
                let next = match event.keystroke.key.as_str() {
                    "left" | "up" => (current + count - 1) % count,
                    "right" | "down" => (current + 1) % count,
                    "home" => 0,
                    "end" => count - 1,
                    _ => return,
                };
                this.prefs.ink = next;
                this.update_appearance(cx);
                cx.stop_propagation();
            }))
            .flex()
            .overflow_x_scroll();
        for (index, name) in names.iter().enumerate() {
            let tooltip = name.clone();
            let mut dots = div().flex().flex_col().gap_1().items_center();
            for shade in crate::theme::ink_shades(index) {
                dots = dots.child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(crate::theme::color(shade)),
                );
            }
            inks = inks.child(
                div()
                    .id(("ink", index))
                    .test_support()
                    .role(Role::RadioButton)
                    .aria_label(name.clone())
                    .aria_toggled(if index == selected {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                            .build(window, cx)
                    })
                    .w(px(24.))
                    .flex_shrink_0()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_lg()
                    .bg(crate::theme::color(if index == selected {
                        p.selection
                    } else {
                        p.panel
                    }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.prefs.ink = index;
                        this.update_appearance(cx);
                    }))
                    .child(dots)
                    .child(
                        div()
                            .absolute()
                            .w(px(0.))
                            .h(px(0.))
                            .overflow_hidden()
                            .child(name.clone()),
                    ),
            );
        }
        div()
            .occlude()
            .absolute()
            .right(px(12.))
            .top(px(self.toolbar_height + 6.))
            .w(px(448.))
            .p_3()
            .rounded_lg()
            .bg(crate::theme::color(p.panel))
            .border_1()
            .border_color(crate::theme::color(p.chrome))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(names[selected].clone())
                    .child(seasons),
            )
            .child(
                div()
                    .p_3()
                    .rounded_lg()
                    .bg(crate::theme::color(p.canvas))
                    .child(
                        div()
                            .text_color(crate::theme::color(p.accent))
                            .child("OpenTelemetry Desktop Viewer"),
                    )
                    .child(
                        div()
                            .text_color(crate::theme::color(p.subtle))
                            .child("Madoromi · Atkinson Hyperlegible Next"),
                    ),
            )
            .child(inks)
            .child(
                div()
                    .text_xs()
                    .text_color(crate::theme::color(p.subtle))
                    .child("Madoromi colours · MPL-2.0"),
            )
            .into_any_element()
    }
    fn update_appearance(&mut self, cx: &mut Context<Self>) {
        crate::theme::selection(self.prefs.season, self.prefs.ink);
        crate::theme::apply(self.prefs.dark, cx);
        self.save_preferences();
        cx.notify();
    }
}

impl Viewer {
    pub fn pane_widths(&self, window: &Window) -> (f32, f32) {
        let available = f32::from(window.viewport_size().width);
        let main = (available * 0.3).max(448.).min(available * 0.65);
        let list = if self.prefs.list_open && self.page != crate::app::Page::Logs {
            self.prefs.list_width.max(160.)
        } else {
            0.
        };
        let details = if self.prefs.details_open {
            self.prefs.details_width.max(200.)
        } else {
            0.
        };
        let budget = (available - main - 16.).max(0.);
        let total = list + details;
        if total > budget && total > 0. {
            (list * budget / total, details * budget / total)
        } else {
            (list, details)
        }
    }
    pub fn pane_resize_handles(&self, window: &Window, cx: &Context<Self>) -> Vec<AnyElement> {
        let (list, details) = self.pane_widths(window);
        let mut handles = Vec::new();
        if list > 0. {
            handles.push(
                div()
                    .id("resize-list")
                    .test_support()
                    .absolute()
                    .top(px(self.toolbar_height))
                    .bottom(px(36.))
                    .left(px(list - 3.))
                    .w(px(6.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.resizing =
                                Some((true, f32::from(event.position.x), this.prefs.list_width));
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element(),
            );
        }
        if details > 0. {
            handles.push(
                div()
                    .id("resize-details")
                    .test_support()
                    .absolute()
                    .top(px(self.toolbar_height))
                    .bottom(px(36.))
                    .right(px(details - 3.))
                    .w(px(6.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.resizing = Some((
                                false,
                                f32::from(event.position.x),
                                this.prefs.details_width,
                            ));
                            cx.stop_propagation();
                        }),
                    )
                    .into_any_element(),
            );
        }
        handles
    }
}

impl Viewer {
    pub fn render_time_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::{button::Button, input::Input};
        let p = crate::theme::palette(self.prefs.dark);
        let mut presets = div().flex().flex_wrap().gap_1();
        for (minutes, label) in [
            (None, "All time"),
            (Some(5), "5m"),
            (Some(15), "15m"),
            (Some(60), "1h"),
            (Some(360), "6h"),
            (Some(1440), "24h"),
            (Some(10080), "7d"),
            (Some(43200), "30d"),
        ] {
            presets = presets.child(
                Button::new(SharedString::from(format!("time-{label}")))
                    .label(label)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_relative_time(minutes, cx)),
                    ),
            );
        }
        div()
            .occlude()
            .absolute()
            .right(px(200.))
            .top(px(self.toolbar_height + 6.))
            .w(px(420.))
            .p_3()
            .bg(crate::theme::color(p.panel))
            .rounded_lg()
            .border_1()
            .border_color(crate::theme::color(p.chrome))
            .flex()
            .flex_col()
            .gap_3()
            .child(presets)
            .child(
                Input::new(&self.timezone_input)
                    .id("timezone")
                    .aria_label("Display timezone"),
            )
            .children(self.timezone_suggestions(cx))
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("timezone-local")
                            .label("Local")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_timezone("Local".into(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("timezone-utc")
                            .label("UTC")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_timezone("UTC".into(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("timezone-apply")
                            .label("Apply timezone")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.change_timezone(
                                    this.timezone_input.read(cx).value().to_string(),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
            .child(
                div()
                    .text_color(crate::theme::color(p.subtle))
                    .child("Absolute range · dates use the chosen timezone"),
            )
            .child(
                Input::new(&self.time_start)
                    .id("time-start")
                    .aria_label("Start time"),
            )
            .child(
                Input::new(&self.time_end)
                    .id("time-end")
                    .aria_label("End time"),
            )
            .child(
                Button::new("apply-absolute-time")
                    .label("Apply absolute range")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let start = parse_time_bound(
                            &this.time_start.read(cx).value(),
                            &this.prefs.timezone,
                        );
                        let end =
                            parse_time_bound(&this.time_end.read(cx).value(), &this.prefs.timezone);
                        match (start, end) {
                            (Ok(start), Ok(end)) => {
                                if start.as_ref().zip(end.as_ref()).is_some_and(|(a, b)| {
                                    a.parse::<i128>().unwrap_or(0) > b.parse::<i128>().unwrap_or(0)
                                }) {
                                    this.status = "Start time must be before end time".into();
                                } else {
                                    this.time_range.start = start;
                                    this.time_range.end = end;
                                    this.time_range.relative_minutes = None;
                                    this.time_range.label = "Absolute range".into();
                                    this.time_open = false;
                                    this.refresh(cx);
                                }
                            }
                            (Err(e), _) | (_, Err(e)) => this.status = e,
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}
fn parse_time_bound(value: &str, timezone: &str) -> Result<Option<String>, String> {
    use chrono::TimeZone;
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if let Ok(ns) = value.parse::<i128>() {
        return Ok(Some(ns.to_string()));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(
            (dt.timestamp() as i128 * 1_000_000_000 + dt.timestamp_subsec_nanos() as i128)
                .to_string(),
        ));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .map_err(|_| "Enter an ISO 8601 timestamp or exact Unix nanoseconds".to_string())?;
    let exact = if timezone == "Local" {
        chrono::Local
            .from_local_datetime(&naive)
            .single()
            .map(|dt| (dt.timestamp(), dt.timestamp_subsec_nanos()))
    } else {
        timezone
            .parse::<chrono_tz::Tz>()
            .ok()
            .and_then(|tz| tz.from_local_datetime(&naive).single())
            .map(|dt| (dt.timestamp(), dt.timestamp_subsec_nanos()))
    };
    exact.map(|(seconds,nanos)|Some((seconds as i128*1_000_000_000+nanos as i128).to_string())).ok_or_else(||"This local time is ambiguous or unavailable in the chosen timezone; include an explicit UTC offset".into())
}
#[cfg(test)]
mod time_tests {
    use super::parse_time_bound;
    #[::core::prelude::v1::test]
    fn local_dates_reject_dst_ambiguity_and_preserve_chosen_zone() {
        assert_eq!(
            parse_time_bound("2026-01-01T00:00:00", "America/Los_Angeles").unwrap(),
            parse_time_bound("2026-01-01T00:00:00-08:00", "UTC").unwrap()
        );
        assert!(parse_time_bound("2026-11-01T01:30:00", "America/Los_Angeles").is_err());
        assert!(parse_time_bound("2026-03-08T02:30:00", "America/Los_Angeles").is_err());
    }
    #[::core::prelude::v1::test]
    fn absolute_bounds_keep_nanoseconds_and_timezone() {
        assert_eq!(
            parse_time_bound("1970-01-01T01:00:00.123456789+01:00", "UTC").unwrap(),
            Some("123456789".into())
        );
        assert_eq!(
            parse_time_bound("9007199254740993", "UTC").unwrap(),
            Some("9007199254740993".into())
        );
        assert_eq!(parse_time_bound("", "UTC").unwrap(), None);
        assert!(parse_time_bound("2026-11-05", "UTC").is_err());
    }
}

impl Viewer {
    fn change_timezone(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        match crate::model::set_timezone(&name) {
            Ok(()) => {
                self.prefs.timezone = name.clone();
                self.timezone_input
                    .update(cx, |input, cx| input.set_value(name, window, cx));
                self.save_preferences();
                self.refresh(cx);
            }
            Err(error) => self.status = format!("Timezone: {error}"),
        }
        cx.notify();
    }
}

impl Viewer {
    fn timezone_suggestions(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        use gpui_kit::component::button::Button;
        let term = self.timezone_input.read(cx).value().to_lowercase();
        if term.len() < 2 || term == "local" || term == "utc" {
            return Vec::new();
        }
        chrono_tz::TZ_VARIANTS
            .iter()
            .filter(|tz| tz.name().to_lowercase().contains(&term))
            .take(6)
            .enumerate()
            .map(|(index, tz)| {
                let name = tz.name().to_string();
                Button::new(("timezone-suggestion", index))
                    .label(name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change_timezone(name.clone(), window, cx)
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

impl Viewer {
    pub fn render_import_report(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::component::button::Button;
        let p = crate::theme::palette(self.prefs.dark);
        let reports = self
            .data
            .get("import.report")
            .cloned()
            .unwrap_or_else(|| json!([]));
        let copy = serde_json::to_string_pretty(&reports).unwrap_or_default();
        let mut panel = div()
            .id("import-results")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Import results")
            .occlude()
            .absolute()
            .right(px(12.))
            .top(px(self.toolbar_height + 6.))
            .w(px((f32::from(window.viewport_size().width) - 24.).min(560.)))
            .max_h(px((f32::from(window.viewport_size().height)
                - self.toolbar_height
                - 60.)
                .max(120.)))
            .overflow_y_scroll()
            .p_3()
            .rounded_lg()
            .bg(crate::theme::color(p.panel))
            .border_1()
            .border_color(crate::theme::color(p.chrome))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child("Import results")
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(Button::new("copy-import-results").label("Copy").on_click(
                                move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                },
                            ))
                            .child(Button::new("close-import-results").label("Close").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.import_report_open = false;
                                    cx.notify();
                                }),
                            )),
                    ),
            );
        if let Some(reports) = reports.as_array() {
            for (file_index, report) in reports.iter().enumerate() {
                let mut section = div().id(("import-file", file_index));
                section = section.child(crate::widgets::heading(
                    crate::model::field(report, "file"),
                    p,
                ));
                if let Some(error) = report["error"].as_str() {
                    section = section.child(crate::widgets::property("Error", error.to_owned(), p));
                }
                if let Some(count) = report["requests"].as_u64() {
                    section = section.child(crate::widgets::property(
                        "Accepted batches",
                        count.to_string(),
                        p,
                    ));
                }
                if let Some(issues) = report["issues"].as_array() {
                    for (index, issue) in issues.iter().enumerate() {
                        section = section.child(crate::widgets::property(
                            format!("Issue {}", index + 1),
                            crate::model::text(issue),
                            p,
                        ));
                    }
                }
                if report["cancelled"] == true {
                    section =
                        section.child(crate::widgets::property("Import status", "Cancelled", p));
                }
                panel = panel.child(section);
            }
        }
        panel.into_any_element()
    }
}

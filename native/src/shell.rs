//! Persistent navigation and responsive search chrome shared by every workspace.
use crate::{
    app::{Page, Viewer},
    theme::{color, palette},
    widgets::button,
};
use gpui_kit::{
    component::{IconName, Selectable, input::Input},
    *,
};
impl Viewer {
    pub fn render_toolbar(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let compact = f32::from(window.viewport_size().width) < 1280.;
        let stats = self.data.get("getStats").cloned().unwrap_or_default();
        let mut navigation = div().flex().items_center().gap_2().child(
            button("list")
                .icon(IconName::PanelLeft)
                .accessibility_label("Toggle List pane")
                .tooltip("Toggle List pane")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.prefs.list_open = !this.prefs.list_open;
                    this.save_preferences();
                    cx.notify();
                })),
        );
        for (page, label, icon, count) in [
            (
                Page::Home,
                "Home",
                IconName::LayoutDashboard,
                self.data
                    .get("home.services")
                    .map(crate::model::discovery_rows)
                    .map(|v| v.len().to_string())
                    .unwrap_or_default(),
            ),
            (
                Page::Traces,
                "Traces",
                IconName::Network,
                crate::model::field(&stats["traces"], "traceCount"),
            ),
            (
                Page::Metrics,
                "Metrics",
                IconName::ChartPie,
                crate::model::field(&stats["metrics"], "metricCount"),
            ),
            (
                Page::Logs,
                "Logs",
                IconName::FileText,
                crate::model::field(&stats["logs"], "logCount"),
            ),
        ] {
            navigation = navigation.child(
                button(label)
                    .icon(icon)
                    .label(if count.is_empty() {
                        label.to_string()
                    } else {
                        format!("{label}  [{count}]")
                    })
                    .role(Role::Tab)
                    .toggled(self.page == page)
                    .selected(self.page == page)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_search_completions(cx);
                        this.page = page;
                        this.refresh(cx);
                    })),
            );
        }
        let theme_group = div()
            .flex()
            .items_center()
            .rounded_full()
            .bg(color(p.panel))
            .child(
                button("appearance")
                    .track_focus(&self.appearance_trigger_focus)
                    .icon(IconName::Palette)
                    .label(["Spring", "Summer", "Autumn", "Winter"][self.prefs.season % 4])
                    .tooltip("Theme and season")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.appearance_focus.focus(window, cx);
                        this.appearance_open = !this.appearance_open;
                        this.time_open = false;
                        cx.notify();
                    })),
            )
            .child(
                button("theme")
                    .icon(if self.prefs.dark {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    })
                    .accessibility_label("Toggle Day and Night")
                    .tooltip("Toggle Day and Night")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prefs.dark = !this.prefs.dark;
                        crate::theme::apply(this.prefs.dark, cx);
                        this.save_preferences();
                        cx.notify();
                    })),
            );
        let appearance = div()
            .flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(theme_group)
            .child(
                button("details")
                    .icon(IconName::PanelRight)
                    .accessibility_label("Toggle Details pane")
                    .tooltip("Toggle Details pane")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prefs.details_open = !this.prefs.details_open;
                        this.save_preferences();
                        cx.notify();
                    })),
            );
        let search_pill = div()
            .flex()
            .items_center()
            .h(px(32.))
            .flex_1()
            .min_w(px(160.))
            .rounded_full()
            .border_1()
            .border_color(color(p.chrome))
            .bg(color(p.panel))
            .child(
                button("refresh")
                    .icon(IconName::RefreshCw)
                    .accessibility_label("Refresh telemetry")
                    .tooltip(if self.new_data {
                        "New telemetry available · Refresh"
                    } else {
                        "Refresh telemetry"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_search_definitions(cx);
                        this.refresh(cx);
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .capture_key_down(cx.listener(|this, event, window, cx| {
                        this.search_key_down(event, window, cx)
                    }))
                    .child(
                        Input::new(&self.search)
                            .id("search")
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .aria_label("Search telemetry"),
                    ),
            )
            .child(
                button("clear-search")
                    .icon(IconName::Close)
                    .accessibility_label("Clear search")
                    .tooltip("Clear search")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.search
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.refresh(cx);
                    })),
            )
            .child(
                button("time-range")
                    .icon(IconName::Calendar)
                    .accessibility_label("Time range")
                    .tooltip(if self.time_range.label.is_empty() {
                        "All time".into()
                    } else {
                        self.time_range.label.clone()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.time_open = !this.time_open;
                        this.appearance_open = false;
                        cx.notify();
                    })),
            );
        let mut search = div()
            .flex()
            .items_center()
            .gap_1()
            .w(px(if compact {
                f32::from(window.viewport_size().width) - 225.
            } else {
                410.
            }))
            .min_w_0()
            .child(search_pill)
            .child(
                button("search-help")
                    .icon(IconName::Info)
                    .accessibility_label("Search help")
                    .tooltip("Search help")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_search_help(window, cx)),
                    ),
            )
            .child(
                button("import")
                    .icon(IconName::FolderOpen)
                    .accessibility_label("Import telemetry JSON")
                    .tooltip("Import telemetry JSON")
                    .on_click(cx.listener(|this, _, window, cx| this.import_files(window, cx))),
            );
        if self.data.contains_key("import.report") {
            search = search.child(
                button("show-import-results")
                    .icon(IconName::FileText)
                    .accessibility_label("Import results")
                    .tooltip("Import results")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.import_report_open = !this.import_report_open;
                        cx.notify();
                    })),
            );
        }
        if self.import_active {
            search = search.child(
                button("cancel-import")
                    .label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.import_cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        this.status = "Cancelling import…".into();
                        cx.notify();
                    })),
            );
        }
        if self
            .backend
            .as_ref()
            .is_none_or(|backend| !backend.is_alive())
            && !self.loading
        {
            search = search.child(
                button("retry-collector")
                    .label("Retry collector")
                    .on_click(cx.listener(|this, _, _, cx| this.start_backend(cx))),
            );
        }
        let mut toolbar = div()
            .id("toolbar")
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .py_2()
            .px_4()
            .bg(color(p.canvas));
        if compact {
            toolbar = toolbar.child(navigation).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(search)
                    .child(appearance),
            );
        } else {
            toolbar = toolbar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(div().flex_1().child(navigation))
                    .child(search)
                    .child(appearance),
            );
        }
        toolbar.into_any_element()
    }
}

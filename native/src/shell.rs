//! Persistent navigation and responsive search chrome shared by every workspace.
use crate::{
    app::{Page, Viewer},
    theme::{color, palette},
};
use gpui_kit::{
    component::{button::Button, input::Input},
    *,
};
impl Viewer {
    pub fn render_toolbar(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let compact = f32::from(window.viewport_size().width) < 1100.;
        let mut navigation = div().flex().items_center().gap_1().child(
            Button::new("list")
                .label("☰")
                .tooltip("Toggle List pane")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.prefs.list_open = !this.prefs.list_open;
                    this.save_preferences();
                    cx.notify();
                })),
        );
        for (page, label) in [
            (Page::Home, "Home"),
            (Page::Traces, "Traces"),
            (Page::Logs, "Logs"),
            (Page::Metrics, "Metrics"),
        ] {
            navigation = navigation.child(
                Button::new(label)
                    .label(label)
                    .role(Role::Tab)
                    .toggled(self.page == page)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.close_search_completions(cx);
                        this.page = page;
                        this.refresh(cx);
                    })),
            )
        }
        let appearance = div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                Button::new("appearance")
                    .track_focus(&self.appearance_trigger_focus)
                    .label("Appearance")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.appearance_focus.focus(window, cx);
                        this.appearance_open = !this.appearance_open;
                        this.time_open = false;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("theme")
                    .label(if self.prefs.dark { "Day" } else { "Night" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prefs.dark = !this.prefs.dark;
                        crate::theme::apply(this.prefs.dark, cx);
                        this.save_preferences();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("details")
                    .label("Details")
                    .toggled(self.prefs.details_open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prefs.details_open = !this.prefs.details_open;
                        this.save_preferences();
                        cx.notify();
                    })),
            );
        let search = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .capture_key_down(cx.listener(|this, event, window, cx| {
                        this.search_key_down(event, window, cx)
                    }))
                    .child(
                        Input::new(&self.search)
                            .id("search")
                            .aria_label("Search telemetry"),
                    ),
            )
            .child(
                Button::new("time-range")
                    .label(if self.time_range.label.is_empty() {
                        "All time".to_owned()
                    } else {
                        self.time_range.label.clone()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.time_open = !this.time_open;
                        this.appearance_open = false;
                        cx.notify();
                    })),
            )
            .children(
                if self
                    .backend
                    .as_ref()
                    .is_none_or(|backend| !backend.is_alive())
                    && !self.loading
                {
                    Some(
                        Button::new("retry-collector")
                            .label("Retry collector")
                            .on_click(cx.listener(|this, _, _, cx| this.start_backend(cx))),
                    )
                } else {
                    None
                },
            )
            .child(
                Button::new("search-help")
                    .label("?")
                    .tooltip("Search help")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_search_help(window, cx)),
                    ),
            )
            .child(
                Button::new("import")
                    .label("Import")
                    .on_click(cx.listener(|this, _, window, cx| this.import_files(window, cx))),
            )
            .children(if self.data.contains_key("import.report") {
                Some(
                    Button::new("show-import-results")
                        .label("Import results")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.import_report_open = !this.import_report_open;
                            cx.notify();
                        })),
                )
            } else {
                None
            })
            .children(if self.import_active {
                Some(
                    Button::new("cancel-import")
                        .label("Cancel import")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.import_cancel
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                            this.status = "Cancelling import…".into();
                            cx.notify();
                        })),
                )
            } else {
                None
            })
            .child(
                Button::new("refresh")
                    .label(if self.new_data {
                        "Refresh •"
                    } else {
                        "Refresh"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_search_definitions(cx);
                        this.refresh(cx)
                    })),
            );
        let mut toolbar = div()
            .id("toolbar")
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(color(p.chrome));
        if compact {
            toolbar = toolbar
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(navigation)
                        .child(appearance),
                )
                .child(search);
        } else {
            toolbar = toolbar.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(navigation)
                    .child(search)
                    .child(appearance),
            );
        }
        toolbar.into_any_element()
    }
}

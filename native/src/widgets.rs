use crate::theme::{Palette, color};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
/// Shared web-style control: compact spacing, a transparent idle surface, full-size text.
pub(crate) fn button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .h(px(32.))
        .text_size(px(14.))
        .font_weight(FontWeight::NORMAL)
}

pub(crate) fn heading(text: impl Into<SharedString>, p: Palette) -> Div {
    div()
        .px_3()
        .py_2()
        .text_color(color(p.subtle))
        .child(text.into())
}
pub(crate) fn empty(text: impl Into<SharedString>, p: Palette) -> Div {
    div().p_5().text_color(color(p.subtle)).child(text.into())
}
pub(crate) fn property(
    name: impl Into<SharedString>,
    value: impl Into<SharedString>,
    p: Palette,
) -> Div {
    let value: SharedString = value.into();
    property_exact(name, value.clone(), value, p)
}
pub(crate) fn property_exact(
    name: impl Into<SharedString>,
    value: impl Into<SharedString>,
    exact: impl Into<SharedString>,
    p: Palette,
) -> Div {
    let name: SharedString = name.into();
    let value: SharedString = value.into();
    let copy = exact.into().to_string();
    let id = SharedString::from(format!("property-copy-{name}"));
    let text_id = SharedString::from(format!("property-text-{name}"));
    let escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\n', "<br>");
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_4()
        .py_2()
        .rounded_lg()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_color(color(p.accent)).child(name.clone()))
                .child(
                    button(id)
                        .icon(gpui_kit::component::IconName::Copy)
                        .accessibility_label(format!("Copy {name}"))
                        .tooltip("Copy exact value")
                        .w(px(20.))
                        .h(px(20.))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                ),
        )
        .child(
            gpui_kit::component::text::TextView::html(text_id, format!("<p>{escaped}</p>"))
                .selectable(true),
        )
}

impl crate::app::Viewer {
    pub fn detail_json(
        &mut self,
        id: &str,
        value: &serde_json::Value,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = crate::theme::palette(self.prefs.dark);
        let children: Vec<(String, serde_json::Value)> = match value {
            serde_json::Value::Object(map) => {
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            }
            serde_json::Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(i, v)| (i.to_string(), v.clone()))
                .collect(),
            _ => Vec::new(),
        };
        if children.is_empty() {
            return div()
                .pl(px(depth as f32 * 8.))
                .py_1()
                .child(
                    gpui_kit::component::text::TextView::html(
                        SharedString::from(id.to_owned()),
                        format!(
                            "<pre>{}</pre>",
                            serde_json::to_string(value)
                                .unwrap_or_default()
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;")
                        ),
                    )
                    .selectable(true),
                )
                .into_any_element();
        }
        let fold_key = format!("json:{id}");
        let open = if depth <= 1 {
            !self.collapsed.contains(&fold_key)
        } else {
            self.collapsed.contains(&fold_key)
        };
        let is_array = value.is_array();
        let label = format!(
            "{} {} [{}]",
            if open { "▾" } else { "▸" },
            if is_array { "[…]" } else { "{…}" },
            children.len()
        );
        let mut view = div().flex().flex_col().pl(px(8.)).child(
            button(SharedString::from(format!("json-button:{id}")))
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.collapsed.remove(&fold_key) {
                        this.collapsed.insert(fold_key.clone());
                    }
                    cx.notify();
                })),
        );
        if open {
            for (key, value) in children {
                view = view.child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_color(crate::theme::color(p.subtle))
                                .child(key.clone()),
                        )
                        .child(self.detail_json(
                            &format!("{id}/{}", key.replace('~', "~0").replace('/', "~1")),
                            &value,
                            depth + 1,
                            cx,
                        )),
                );
            }
        }
        view.into_any_element()
    }
}

impl crate::app::Viewer {
    /// Received OTel attributes are named typed fields, not an Array/Object inspector.
    pub fn attribute_table(
        &mut self,
        id: &str,
        value: &serde_json::Value,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = crate::theme::palette(self.prefs.dark);
        let mut rows = div()
            .id(SharedString::from(format!("attributes-{id}")))
            .test_support()
            .flex()
            .flex_col()
            .min_w_0();
        if let Some(items) = value.as_array() {
            for (index, item) in items.iter().enumerate() {
                let name = crate::model::field(item, "key");
                let kind = crate::model::field(&item["value"], "kind");
                let display = crate::model::attribute(&item["value"]);
                rows = rows.child(div().id(("attribute", index)).child(property_exact(
                    format!("{name} · {kind}"),
                    display.clone(),
                    display,
                    p,
                )));
            }
        } else if let Some(object) = value.as_object() {
            for (key, field) in object {
                if key == "attributes" {
                    rows = rows.child(self.attribute_table(&format!("{id}/attributes"), field, cx));
                } else if !field.is_null() {
                    rows = rows.child(property(key.clone(), crate::model::text(field), p));
                }
            }
        } else {
            rows = rows.child(property("Value", crate::model::text(value), p));
        }
        rows.into_any_element()
    }
}

impl crate::app::Viewer {
    pub fn workspace_stacked(&self, window: &Window) -> bool {
        f32::from(window.viewport_size().width) < 1280.
            && self.page != crate::app::Page::Logs
            && self.prefs.list_open
            && self.prefs.details_open
    }
    pub fn status_height(&self) -> f32 {
        if self.status.is_empty()
            || self.status.starts_with("Collector ready")
            || self.status.starts_with("Import finished")
        {
            0.
        } else {
            34.
        }
    }
    pub fn stacked_main_height(&self, window: &Window) -> f32 {
        let available = (f32::from(window.viewport_size().height)
            - self.toolbar_height
            - self.status_height()
            - 32.
            - 16.)
            .max(280.);
        let tall_main = matches!(
            self.page,
            crate::app::Page::Home | crate::app::Page::Metrics
        );
        let desired = if tall_main && (self.prefs.stacked_main_height - 240.).abs() < 0.1 {
            360.
        } else {
            self.prefs.stacked_main_height
        };
        desired.clamp(
            if tall_main { 320. } else { 140. },
            (available - 180.).max(if tall_main { 320. } else { 140. }),
        )
    }
    pub fn render_workspace(
        &self,
        list: Option<AnyElement>,
        main: AnyElement,
        details: Option<AnyElement>,
        window: &Window,
    ) -> AnyElement {
        let stacked = self.workspace_stacked(window) && list.is_some() && details.is_some();
        let (list_width, details_width) = self.pane_widths(window);
        let mut workspace = div()
            .id("workspace-layout")
            .test_support()
            .flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .gap_4();
        if let Some(list) = list {
            workspace = workspace.child(
                div()
                    .id("workspace-list")
                    .test_support()
                    .w(px(list_width))
                    .flex_shrink_0()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .overflow_hidden()
                    .child(list),
            );
        }
        if stacked {
            workspace = workspace.child(
                div()
                    .id("workspace-main-details")
                    .test_support()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .gap_4()
                    .child(
                        div()
                            .id("workspace-main")
                            .test_support()
                            .h(px(self.stacked_main_height(window)))
                            .flex_shrink_0()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(main),
                    )
                    .child(
                        div()
                            .id("workspace-details")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_hidden()
                            .child(details.unwrap()),
                    ),
            );
        } else {
            workspace = workspace.child(
                div()
                    .id("workspace-main")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .overflow_hidden()
                    .child(main),
            );
            if let Some(details) = details {
                workspace = workspace.child(
                    div()
                        .id("workspace-details")
                        .test_support()
                        .w(px(details_width))
                        .flex_shrink_0()
                        .min_w_0()
                        .min_h_0()
                        .h_full()
                        .overflow_hidden()
                        .child(details),
                );
            }
        }
        workspace.into_any_element()
    }
}

pub(crate) fn relative_timestamp(value: &serde_json::Value) -> String {
    let delta = (chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default() as i128
        - crate::model::ns(value))
        / 1_000_000_000;
    if delta < 60 {
        return "just now".into();
    }
    for (unit, seconds) in [
        ("year", 31_536_000),
        ("month", 2_592_000),
        ("week", 604_800),
        ("day", 86_400),
        ("hour", 3600),
        ("minute", 60),
    ] {
        if delta >= seconds {
            let n = delta / seconds;
            return format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
        }
    }
    "just now".into()
}

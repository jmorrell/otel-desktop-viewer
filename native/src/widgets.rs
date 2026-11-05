use crate::theme::{Palette, color};
use gpui_kit::*;
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
        .p_3()
        .border_b_1()
        .border_color(color(p.chrome))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().text_color(color(p.subtle)).child(name))
                .child(
                    gpui_kit::component::button::Button::new(id)
                        .label("Copy")
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
            if is_array { "Array" } else { "Object" },
            children.len()
        );
        let mut view = div().flex().flex_col().pl(px(8.)).child(
            gpui_kit::component::button::Button::new(SharedString::from(format!(
                "json-button:{id}"
            )))
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

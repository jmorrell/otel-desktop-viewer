use crate::{
    app::Viewer,
    theme::{color, palette},
};
use gpui_kit::{component::button::Button, *};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct SectionPreference {
    pub id: String,
    pub height: f32,
    pub collapsed: bool,
}
pub struct DetailSection {
    pub id: String,
    pub title: String,
    pub content: AnyElement,
}
impl DetailSection {
    pub fn new(id: impl Into<String>, title: impl Into<String>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            content: content.into_any_element(),
        }
    }
}
impl Viewer {
    pub fn detail_stack(
        &mut self,
        signal: &str,
        sections: Vec<DetailSection>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let layout = self.prefs.detail_layout.entry(signal.into()).or_default();
        for section in &sections {
            if !layout.iter().any(|v| v.id == section.id) {
                layout.push(SectionPreference {
                    id: section.id.clone(),
                    height: 300.,
                    collapsed: false,
                });
            }
        }
        // Canonical section order comes from the view; preferences store size/state only.
        let prefs = sections
            .iter()
            .filter_map(|section| layout.iter().find(|p| p.id == section.id).cloned())
            .collect::<Vec<_>>();
        let mut sections: std::collections::HashMap<_, _> =
            sections.into_iter().map(|s| (s.id.clone(), s)).collect();
        let mut stack = div().flex().flex_col().min_w_0();
        for (index, pref) in prefs.iter().enumerate() {
            let Some(section) = sections.remove(&pref.id) else {
                continue;
            };
            let signal = signal.to_owned();
            let id = pref.id.clone();
            let key = format!("{signal}:{}", pref.id);
            let collapsed = pref.collapsed;
            let collapse_signal = signal.clone();
            let collapse_id = id.clone();
            let resize_signal = signal.clone();
            let resize_id = id.clone();
            let mut frame = div()
                .id(SharedString::from(format!("detail-section-{key}")))
                .test_support()
                .min_w_0()
                .border_b_1()
                .border_color(color(p.chrome))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .child(
                            Button::new(SharedString::from(format!("detail-collapse-{key}")))
                                .label(if collapsed { "▸" } else { "▾" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if index > 0 {
                                        if let Some(s) = this
                                            .prefs
                                            .detail_layout
                                            .get_mut(&collapse_signal)
                                            .and_then(|v| {
                                                v.iter_mut().find(|s| s.id == collapse_id)
                                            })
                                        {
                                            s.collapsed = !s.collapsed;
                                        }
                                    }
                                    this.save_preferences();
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1().child(section.title)),
                );
            if !collapsed {
                frame = frame
                    .child(
                        div()
                            .id(SharedString::from(format!("detail-body-{key}")))
                            .h(px(pref.height.clamp(80., 1200.)))
                            .overflow_y_scroll()
                            .min_w_0()
                            .child(section.content),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("detail-resize-{key}")))
                            .test_support()
                            .h(px(7.))
                            .w_full()
                            .cursor_row_resize()
                            .bg(color(p.chrome))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                    this.detail_resizing = Some((
                                        resize_signal.clone(),
                                        resize_id.clone(),
                                        f32::from(e.position.y),
                                        this.prefs
                                            .detail_layout
                                            .get(&resize_signal)
                                            .and_then(|v| v.iter().find(|s| s.id == resize_id))
                                            .map(|s| s.height)
                                            .unwrap_or(300.),
                                    ));
                                    cx.stop_propagation();
                                }),
                            ),
                    );
            }
            stack = stack.child(frame);
        }
        stack.into_any_element()
    }
}

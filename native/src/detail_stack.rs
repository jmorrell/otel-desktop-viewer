use crate::{
    app::Viewer,
    theme::{color, palette},
};
use gpui_kit::{
    component::{Disableable, IconName},
    *,
};
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
    pub actions: Option<AnyElement>,
    pub initially_open: bool,
    pub available: bool,
}
impl DetailSection {
    pub fn new(id: impl Into<String>, title: impl Into<String>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            content: content.into_any_element(),
            actions: None,
            initially_open: true,
            available: true,
        }
    }
}
impl DetailSection {
    pub fn actions(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(actions.into_any_element());
        self
    }
    pub fn available(mut self, available: bool) -> Self {
        self.available = available;
        self
    }
    pub fn initially_open(mut self, open: bool) -> Self {
        self.initially_open = open;
        self
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
                    collapsed: !section.initially_open,
                });
            }
        }
        // Canonical section order comes from the view; preferences store size/state only.
        layout.retain(|p| sections.iter().any(|s| s.id == p.id));
        layout.sort_by_key(|p| {
            sections
                .iter()
                .position(|s| s.id == p.id)
                .unwrap_or(usize::MAX)
        });
        let prefs = sections
            .iter()
            .filter_map(|section| {
                layout
                    .iter()
                    .find(|p| p.id == section.id)
                    .cloned()
                    .map(|mut pref| {
                        if !section.available {
                            pref.collapsed = true;
                        }
                        pref
                    })
            })
            .collect::<Vec<_>>();
        let total = prefs
            .iter()
            .filter(|p| !p.collapsed)
            .map(|p| p.height)
            .sum::<f32>()
            .max(1.);
        let body_space = (self.details_available_height
            - prefs.len() as f32 * 48.
            - (prefs.len().saturating_sub(1)) as f32 * 4.
            - prefs.iter().filter(|p| !p.collapsed).count() as f32 * 4.)
            .max(80.);
        let scale = body_space / total;
        self.detail_scales.insert(signal.into(), scale);
        self.detail_open.insert(
            signal.into(),
            prefs
                .iter()
                .filter(|p| !p.collapsed)
                .map(|p| p.id.clone())
                .collect(),
        );
        let mut sections: std::collections::HashMap<_, _> =
            sections.into_iter().map(|s| (s.id.clone(), s)).collect();
        let mut stack = div().flex().flex_col().min_w_0().gap_1();
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_4()
                        .h(px(48.))
                        .rounded_lg()
                        .bg(color(if collapsed { p.selection } else { p.panel }))
                        .child(
                            crate::widgets::button(SharedString::from(format!(
                                "detail-collapse-{key}"
                            )))
                            .icon(if index == 0 {
                                IconName::Network
                            } else if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .disabled(index == 0 || !section.available)
                            .accessibility_label(format!("Toggle {}", section.title))
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
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
                                },
                            )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .child(section.title),
                        )
                        .children(section.actions),
                );
            if !collapsed {
                frame = frame
                    .child(
                        div()
                            .id(SharedString::from(format!("detail-body-{key}")))
                            .test_support()
                            .role(Role::Group)
                            .h(px((pref.height * scale).max(40.)))
                            .overflow_y_scroll()
                            .min_w_0()
                            .child(section.content),
                    )
                    .children(if prefs.iter().skip(index + 1).any(|s| !s.collapsed) {
                        Some(
                            div()
                                .id(SharedString::from(format!("detail-resize-{key}")))
                                .test_support()
                                .h(px(4.))
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
                        )
                    } else {
                        None
                    });
            }
            stack = stack.child(frame);
        }
        stack.into_any_element()
    }
}

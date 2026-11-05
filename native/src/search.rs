//! Search completion and help for the native search box.
use crate::{
    app::{Page, Viewer},
    model::{attribute, field},
    theme::{color, palette},
};
use gpui_kit::{
    component::{button::Button, input::Position},
    *,
};
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Default)]
pub struct SearchState {
    open: bool,
    help: bool,
    selected: usize,
    definitions_for: String,
    term: String,
    value_key: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Suggestion {
    label: String,
    insert: String,
    start: usize,
    end: usize,
}
fn signal(page: Page) -> &'static str {
    match page {
        Page::Traces => "traces",
        Page::Logs => "logs",
        Page::Metrics => "metrics",
        Page::Home => "all",
    }
}
fn token_range(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(text.len());
    let cursor = (0..=cursor)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    let start = text[..cursor]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace() || "()[],=<>!|".contains(*c))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let end = text[cursor..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || "()[],=<>!|".contains(*c))
        .map(|(i, _)| cursor + i)
        .unwrap_or(text.len());
    (start, end)
}
fn value_context(text: &str, start: usize) -> Option<&str> {
    let before = text[..start].trim_end();
    let mut words = before.split_whitespace().rev();
    let op = words.next()?;
    let name = words.next()?;
    if ["=", "!=", ">", "<", ">=", "<=", "CONTAINS", "REGEXP"]
        .iter()
        .any(|o| op.eq_ignore_ascii_case(o))
    {
        Some(name)
    } else {
        None
    }
}
fn field_supported(signal: &str, name: &str) -> bool {
    matches!(
        (signal, name),
        ("traces", "name") | ("logs", "eventName") | ("metrics", "name") | ("metrics", "unit")
    )
}
fn allowed_owner(signal: &str, owner: &str) -> bool {
    match signal {
        "traces" => ["resource", "scope", "span", "event", "link"].contains(&owner),
        "logs" => ["resource", "scope", "log"].contains(&owner),
        "metrics" => ["resource", "scope", "datapoint", "exemplar", "metadata"].contains(&owner),
        _ => true,
    }
}
fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}
fn attr_expression(def: &Value) -> Option<String> {
    let owner = def["attributeScope"].as_str()?;
    let name = def["name"].as_str()?;
    let kind = def["type"].as_str()?;
    if ![
        "resource",
        "scope",
        "span",
        "event",
        "link",
        "log",
        "datapoint",
        "exemplar",
        "metadata",
    ]
    .contains(&owner)
    {
        return None;
    };
    Some(format!("attr({owner}, {}, {kind})", quote(name)))
}
fn apply_edit(value: &str, suggestion: &Suggestion) -> Option<(String, u32)> {
    if suggestion.start > suggestion.end
        || suggestion.end > value.len()
        || !value.is_char_boundary(suggestion.start)
        || !value.is_char_boundary(suggestion.end)
    {
        return None;
    }
    let mut next = String::with_capacity(value.len() + suggestion.insert.len());
    next.push_str(&value[..suggestion.start]);
    next.push_str(&suggestion.insert);
    let caret = next.encode_utf16().count() as u32;
    next.push_str(&value[suggestion.end..]);
    Some((next, caret))
}
fn literal(v: &Value) -> Option<String> {
    match v["kind"].as_str()? {
        "string" => Some(quote(v["value"].as_str()?)),
        "int64" | "double" | "bool" => Some(attribute(v)),
        _ => None,
    }
}
impl Viewer {
    pub fn refresh_search_definitions(&mut self, cx: &mut Context<Self>) {
        let sig = signal(self.page);
        if sig == "all" || self.backend.is_none() {
            return;
        }
        self.search_state.definitions_for = sig.to_owned();
        // Force the next update to refresh value-first and field-value matches too.
        self.search_state.term = "\0".into();
        self.search_state.value_key.clear();
        let method = match sig {
            "traces" => "getTraceAttributeDefinitions",
            "logs" => "getLogAttributeDefinitions",
            _ => "getMetricAttributeDefinitions",
        };
        self.rpc_key(&format!("search.definitions.{sig}"), method, json!([]), cx);
    }
    pub fn search_changed(&mut self, cx: &mut Context<Self>) {
        self.search_state.help = false;
        self.update_search_completions(cx);
    }
    pub fn update_search_completions(&mut self, cx: &mut Context<Self>) {
        let input = self.search.read(cx);
        let text = input.value().to_string();
        let cursor = input.cursor();
        let (start, end) = token_range(&text, cursor);
        let term = text[start..cursor.min(end)]
            .trim_matches(['\'', '"'])
            .to_owned();
        let sig = signal(self.page).to_owned();
        let field = value_context(&text, start).unwrap_or("").to_owned();
        self.search_state.open = true;
        self.search_state.selected = 0;
        if sig != "all" && self.backend.is_some() && self.search_state.definitions_for != sig {
            self.refresh_search_definitions(cx);
        }
        if self.search_state.term != term {
            self.search_state.term = term.clone();
            self.data.remove("search.matches");
            if term.chars().count() >= 2 {
                self.rpc_key(
                    "search.matches",
                    "searchAttributeMatches",
                    json!({"term":term}),
                    cx,
                );
            } else {
                *self.generations.entry("search.matches".into()).or_default() += 1;
            }
        }
        let key = format!("{sig}\0{field}\0{term}");
        if self.search_state.value_key != key {
            self.search_state.value_key = key;
            self.data.remove("search.values");
            if field_supported(&sig, &field) {
                self.rpc_key(
                    "search.values",
                    "getFieldValueCompletions",
                    json!({"signal":sig,"field":field,"term":term,"limit":30}),
                    cx,
                );
            } else {
                *self.generations.entry("search.values".into()).or_default() += 1;
            }
        }
        cx.notify();
    }
    fn search_suggestions(&self, cx: &App) -> Vec<Suggestion> {
        let input = self.search.read(cx);
        let text = input.value().to_string();
        let cursor = input.cursor();
        let (start, end) = token_range(&text, cursor);
        let typed = text[start..cursor.min(end)].trim_matches(['\'', '"']);
        let lower = typed.to_lowercase();
        let sig = signal(self.page);
        let value_field = value_context(&text, start);
        if typed.is_empty() && value_field.is_none() {
            return Vec::new();
        }
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        if value_field.is_some() {
            if let Some(values) = self.data.get("search.values").and_then(Value::as_array) {
                for v in values {
                    if let Some(v) = v.as_str() {
                        let insert = quote(v);
                        if seen.insert(insert.clone()) {
                            result.push(Suggestion {
                                label: format!("Value · {v}"),
                                insert,
                                start,
                                end,
                            });
                        }
                    }
                }
            }
        } else {
            let fields: Value =
                serde_json::from_str(include_str!("fields.json")).unwrap_or(Value::Null);
            let categories: &[&str] = match sig {
                "traces" => &["RESOURCE", "SCOPE", "SPAN"],
                "logs" => &["RESOURCE", "SCOPE", "LOG"],
                "metrics" => &["RESOURCE", "SCOPE", "METRIC"],
                _ => &["RESOURCE", "SCOPE", "SPAN", "LOG", "METRIC"],
            };
            for category in categories {
                if let Some(list) = fields[*category].as_array() {
                    for f in list {
                        let name = field(f, "name");
                        if !name.is_empty()
                            && name != "serviceIdentity"
                            && name.to_lowercase().contains(&lower)
                            && seen.insert(format!("field:{name}"))
                        {
                            if result.len() >= 6 {
                                continue;
                            }
                            result.push(Suggestion {
                                label: format!("Field · {name} ({})", field(f, "type")),
                                insert: format!("{name} "),
                                start,
                                end,
                            });
                        }
                    }
                }
            }
            if let Some(defs) = self
                .data
                .get(&format!("search.definitions.{sig}"))
                .and_then(Value::as_array)
            {
                for def in defs {
                    let owner = field(def, "attributeScope");
                    let name = field(def, "name");
                    if allowed_owner(sig, &owner) && name.to_lowercase().contains(&lower) {
                        if let Some(expr) = attr_expression(def) {
                            if seen.insert(expr.clone()) && result.len() < 9 {
                                result.push(Suggestion {
                                    label: format!(
                                        "Attribute · {owner}.{name} ({})",
                                        field(def, "type")
                                    ),
                                    insert: format!("{expr} "),
                                    start,
                                    end,
                                });
                            }
                        }
                    }
                }
            }
            if let Some(matches) = self.data.get("search.matches").and_then(Value::as_array) {
                for def in matches {
                    let owner = field(def, "attributeScope");
                    if !allowed_owner(sig, &owner) {
                        continue;
                    }
                    if let Some(expr) = attr_expression(def) {
                        if let Some(samples) = def["sampleValues"].as_array() {
                            for value in samples.iter().take(2) {
                                if let Some(value) = literal(value) {
                                    let insert = format!("{expr} = {value}");
                                    if seen.insert(insert.clone()) {
                                        result.push(Suggestion {
                                            label: format!(
                                                "Value match · {owner}.{} = {value}",
                                                field(def, "name")
                                            ),
                                            insert,
                                            start,
                                            end,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result.truncate(12);
        result
    }
    fn apply_search_suggestion(
        &mut self,
        suggestion: Suggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.search.read(cx).value().to_string();
        let Some((next, caret)) = apply_edit(&value, &suggestion) else {
            return;
        };
        self.search.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.set_cursor_position(Position::new(0, caret), window, cx);
        });
        self.search_state.open = false;
        self.search_state.selected = 0;
        cx.notify();
    }
    pub fn search_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        if key == "escape" && (self.search_state.open || self.search_state.help) {
            self.search_state.open = false;
            self.search_state.help = false;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if !self.search_state.open {
            return;
        }
        let choices = self.search_suggestions(cx);
        if choices.is_empty() {
            return;
        }
        match key {
            "down" => {
                self.search_state.selected = (self.search_state.selected + 1) % choices.len();
                cx.stop_propagation();
                cx.notify();
            }
            "up" => {
                self.search_state.selected =
                    (self.search_state.selected + choices.len() - 1) % choices.len();
                cx.stop_propagation();
                cx.notify();
            }
            "enter" | "tab" => {
                let index = self.search_state.selected.min(choices.len() - 1);
                let suggestion = choices[index].clone();
                let input = self.search.read(cx);
                let text = input.value().to_string();
                let cursor = input.cursor();
                let typed =
                    text[suggestion.start..cursor.min(suggestion.end)].trim_matches(['\'', '"']);
                let proposed = suggestion.insert.trim().trim_matches('"');
                let complete = key == "enter"
                    && typed.eq_ignore_ascii_case(proposed)
                    && crate::model::parse_search_request_with_fields(
                        &text,
                        signal(self.page),
                        &self.current_search_definitions(),
                    )
                    .is_ok();
                if complete {
                    self.submit_search(window, cx);
                } else {
                    self.apply_search_suggestion(suggestion, window, cx);
                }
                cx.stop_propagation();
            }
            _ => {}
        }
    }
    pub fn current_search_definitions(&self) -> Vec<Value> {
        self.data
            .get(&format!("search.definitions.{}", signal(self.page)))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
    pub fn close_search_completions(&mut self, cx: &mut Context<Self>) {
        self.search_state.open = false;
        self.search_state.help = false;
        cx.notify();
    }
    pub fn submit_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.search_state.open = false;
        self.search_state.help = false;
        self.refresh(cx);
    }
    pub fn toggle_search_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_state.help = !self.search_state.help;
        self.search_state.open = false;
        if self.search_state.help {
            self.search.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }
    pub fn render_search_completions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = palette(self.prefs.dark);
        if !self.search_state.help && !self.search_state.open {
            return None;
        }
        let choices = if self.search_state.help {
            Vec::new()
        } else {
            self.search_suggestions(cx)
        };
        if !self.search_state.help && choices.is_empty() {
            return None;
        }
        let viewport = f32::from(window.viewport_size().width);
        let width = (viewport - 32.).clamp(200., 600.);
        let caret_x = self
            .search
            .read(cx)
            .cursor_layout()
            .map(|(bounds, _)| f32::from(bounds.origin.x))
            .unwrap_or(16.);
        let left = caret_x.max(16.).min((viewport - width - 8.).max(0.));
        let mut panel = div()
            .id("search-completions")
            .absolute()
            .top(px(0.))
            .left(px(left))
            .w(px(width))
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(400.))
            .overflow_y_scroll()
            .occlude()
            .bg(color(p.panel))
            .border_1()
            .border_color(color(p.chrome))
            .rounded_lg()
            .shadow_lg()
            .p_2();
        if self.search_state.help {
            panel = panel
                .child(div().child("Search help"))
                .child("Search text, or combine field conditions with AND, OR, and parentheses.")
                .child("Examples: severityNumber >= 17 · duration >= 1.5ms · name = \"checkout\"")
                .child("Attributes: attr(resource, \"service.name\", string) = \"checkout\"")
                .child(
                    "Use IN, NOT IN, CONTAINS, REGEXP, and NULL. Append | LIMIT 50 to cap results.",
                )
                .child("Choose suggestions with ↑/↓ and Enter; Escape closes.");
        } else {
            for (i, suggestion) in choices.into_iter().enumerate() {
                let selected = i == self.search_state.selected;
                let chosen = suggestion.clone();
                panel = panel.child(
                    Button::new(("search-suggestion", i))
                        .label(suggestion.label)
                        .toggled(selected)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_search_suggestion(chosen.clone(), window, cx)
                        })),
                );
            }
        }
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(self.toolbar_height))
                .bottom_0()
                .child(div().absolute().inset_0().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.close_search_completions(cx)),
                ))
                .child(panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Suggestion, apply_edit, attr_expression, field_supported, token_range, value_context,
    };
    use serde_json::json;
    #[test]
    fn utf8_cursor_and_context_are_safe() {
        let text = "name = 付款";
        let (start, end) = token_range(text, text.len());
        assert_eq!(&text[start..end], "付款");
        assert_eq!(value_context(text, start), Some("name"));
        assert!(field_supported("traces", "name"));
        assert!(!field_supported("logs", "name"));
    }
    #[test]
    fn completing_mid_query_preserves_suffix_and_utf16_cursor() {
        let input = "🌱 AND sta OR name = \"x\"";
        let start = input.find("sta").unwrap();
        let suggestion = Suggestion {
            label: String::new(),
            insert: "statusCode ".into(),
            start,
            end: start + 3,
        };
        let (result, caret) = apply_edit(input, &suggestion).unwrap();
        assert_eq!(result, "🌱 AND statusCode  OR name = \"x\"");
        assert_eq!(caret, "🌱 AND statusCode ".encode_utf16().count() as u32);
    }
    #[test]
    fn attributes_use_explicit_typed_search_syntax() {
        assert_eq!(
            attr_expression(
                &json!({"name":"service.name","attributeScope":"resource","type":"string"})
            )
            .as_deref(),
            Some("attr(resource, \"service.name\", string)")
        );
    }
}

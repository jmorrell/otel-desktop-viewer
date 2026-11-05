use crate::{
    app::Viewer,
    model::*,
    theme::{color, palette, service_color},
    widgets,
};
use gpui_kit::{
    component::{Selectable, input::Input},
    *,
};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

const SIGNALS: [&str; 3] = ["traces", "logs", "metrics"];
const PAGE_SIZES: [usize; 3] = [25, 50, 100];

fn state_text(data: &HashMap<String, Value>, key: &str, fallback: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_owned()
}

fn state_bool(data: &HashMap<String, Value>, key: &str, fallback: bool) -> bool {
    data.get(key).and_then(Value::as_bool).unwrap_or(fallback)
}

fn page_size(data: &HashMap<String, Value>, signal: &str) -> usize {
    data.get(&format!("home.page-size.{signal}"))
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| PAGE_SIZES.contains(n))
        .unwrap_or(25)
}

fn page_offset(data: &HashMap<String, Value>, signal: &str) -> usize {
    data.get(&format!("home.offset.{signal}"))
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

fn owner_options(signal: &str) -> &'static [&'static str] {
    match signal {
        "traces" => &["resource", "scope", "span", "event", "link"],
        "logs" => &["resource", "scope", "log"],
        _ => &["resource", "scope", "datapoint", "exemplar", "metadata"],
    }
}

fn service_identity(service: &Value) -> String {
    field(service, "serviceIdentity")
}

fn selected_identity(data: &HashMap<String, Value>) -> Option<String> {
    data.get("home.selected-identity")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn home_service_pages(data: &HashMap<String, Value>) -> Vec<(usize, Value)> {
    let mut pages = Vec::new();
    if let Some(page) = data.get("home.services") {
        pages.push((0, page.clone()));
    }
    for (key, page) in data {
        let Some(offset) = key
            .strip_prefix("home.services.more.")
            .and_then(|offset| offset.parse::<usize>().ok())
        else {
            continue;
        };
        pages.push((offset, page.clone()));
    }
    pages.sort_by_key(|(offset, _)| *offset);
    pages
}

fn home_service_rows(data: &HashMap<String, Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    home_service_pages(data)
        .into_iter()
        .flat_map(|(_, page)| discovery_rows(&page))
        .filter(|service| seen.insert(service_identity(service)))
        .collect()
}

fn home_services_have_more(data: &HashMap<String, Value>) -> bool {
    home_service_pages(data)
        .last()
        .is_some_and(|(_, page)| page["truncated"] == true)
}

fn next_home_services_offset(data: &HashMap<String, Value>) -> usize {
    home_service_pages(data)
        .last()
        .map(|(offset, page)| *offset + discovery_rows(page).len())
        .unwrap_or(0)
}

fn attribute_entry_id(identity: &str, signal: &str, owner: &str, key: &Value) -> String {
    let key_name = field(key, "key");
    let key_kind = field(key, "kind");
    serde_json::to_string(&[
        identity,
        signal,
        owner,
        key_name.as_str(),
        key_kind.as_str(),
    ])
    .unwrap_or_default()
}

fn attribute_entry_expanded(data: &HashMap<String, Value>, id: &str) -> bool {
    data.get(&format!("home.expanded-override.{id}"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn service_label(service: &Value) -> String {
    let name = field(service, "serviceName");
    let namespace = field(service, "serviceNamespace");
    let name_kind = field(service, "serviceNameKind");
    let namespace_kind = field(service, "serviceNamespaceKind");
    let name = if name.is_empty() {
        format!("(unnamed · {name_kind})")
    } else if name_kind != "string" && !name_kind.is_empty() {
        format!("{name} · {name_kind}")
    } else {
        name
    };
    if namespace.is_empty() {
        name
    } else if namespace_kind != "string" && !namespace_kind.is_empty() {
        format!("{namespace} / {name} · {namespace_kind}")
    } else {
        format!("{namespace} / {name}")
    }
}

fn service_count(service: &Value, signal: &str) -> u64 {
    let key = match signal {
        "traces" => "traceCount",
        "logs" => "logCount",
        _ => "metricCount",
    };
    service[key].as_u64().unwrap_or(0)
}

fn display_count(service: &Value, signal: &str) -> String {
    let (label, value) = match signal {
        "traces" => ("traces", service["traceCount"].as_u64().unwrap_or(0)),
        "logs" => ("logs", service["logCount"].as_u64().unwrap_or(0)),
        _ => ("metrics", service["metricCount"].as_u64().unwrap_or(0)),
    };
    let label = if value == 1 {
        label.trim_end_matches('s')
    } else {
        label
    };
    format!("{value} {label}")
}

fn service_sort(rows: &mut [Value], sort: &str, direction: &str) {
    rows.sort_by(|a, b| {
        let ordering = match sort {
            "traces" => a["traceCount"].as_u64().cmp(&b["traceCount"].as_u64()),
            "logs" => a["logCount"].as_u64().cmp(&b["logCount"].as_u64()),
            "metrics" => a["metricCount"].as_u64().cmp(&b["metricCount"].as_u64()),
            "lastSeen" => ns(&a["lastSeen"]).cmp(&ns(&b["lastSeen"])),
            _ => service_label(a)
                .to_lowercase()
                .cmp(&service_label(b).to_lowercase()),
        };
        let ordering = if direction == "asc" {
            ordering
        } else {
            ordering.reverse()
        };
        ordering.then_with(|| service_identity(a).cmp(&service_identity(b)))
    });
}

fn record_title(record: &Value, signal: &str) -> String {
    match signal {
        "traces" => {
            let name = field(&record["rootSpan"], "name");
            if name.is_empty() {
                "Trace without a root span".into()
            } else {
                name
            }
        }
        "logs" => {
            let body = field(record, "bodyPreview");
            if body.is_empty() {
                "Log without a body".into()
            } else {
                body
            }
        }
        _ => field(record, "name"),
    }
}

fn record_subtitle(record: &Value, signal: &str) -> String {
    match signal {
        "traces" => format!(
            "{} spans · {} errors · {}",
            field(record, "spanCount"),
            field(record, "errorCount"),
            format_duration(ns(&record["durationNs"]))
        ),
        "logs" => format!(
            "{} · {}",
            field(record, "severityText"),
            format_timestamp(ns(&record["timestamp"]))
        ),
        _ => format!(
            "{} · {} series · {} points",
            field(record, "metricType"),
            field(record, "seriesCount"),
            field(record, "datapointCount")
        ),
    }
}

fn total_label(service: &Value, signal: &str, rows_len: usize) -> String {
    let total = match signal {
        "traces" => service["traceCount"].as_u64().unwrap_or(rows_len as u64),
        "logs" => service["logCount"].as_u64().unwrap_or(rows_len as u64),
        _ => service["metricCount"].as_u64().unwrap_or(rows_len as u64),
    };
    total.to_string()
}

fn owner_and_signal(data: &HashMap<String, Value>) -> (String, String) {
    let signal = state_text(data, "home.discovery-signal", "traces");
    let owner = state_text(data, &format!("home.discovery-owner.{signal}"), "resource");
    (signal, owner)
}

fn telemetry_query(identity: &str, scope: Option<&Value>, search: Option<&Value>) -> Value {
    let service = condition(
        json!({"name":"serviceIdentity","type":"string","searchScope":"field"}),
        "=",
        identity,
    );
    let mut children = vec![service];
    if let Some(search) = search.filter(|query| !query.is_null()) {
        children.push(search.clone());
    }
    let Some(scope) = scope else {
        return json!({"id":"home-service-query","type":"group","group":{"logicalOperator":"AND","children":children}});
    };
    let scoped = condition(
        json!({
            "name": field(scope, "key"),
            "type": field(scope, "kind"),
            "searchScope": "attribute",
            "attributeScope": field(scope, "ownerType")
        }),
        "=",
        &attribute(&scope["value"]),
    );
    children.push(scoped);
    json!({"id":"home-service-scope","type":"group","group":{"logicalOperator":"AND","children":children}})
}

fn home_queries(input: &str) -> (Value, Option<String>) {
    let mut queries = serde_json::Map::new();
    let mut errors = Vec::new();
    for signal in SIGNALS {
        if input.trim().is_empty() {
            queries.insert(signal.into(), Value::Null);
        } else {
            match parse_search(input, signal) {
                Ok(query) => {
                    queries.insert(signal.into(), query);
                }
                Err(error) => {
                    queries.insert(signal.into(), json!(false));
                    errors.push(format!("{signal}: {error}"));
                }
            }
        }
    }
    let all_invalid = !input.trim().is_empty() && errors.len() == SIGNALS.len();
    (
        Value::Object(queries),
        all_invalid.then(|| errors.join(" · ")),
    )
}

fn service_map_scale(data: &HashMap<String, Value>) -> f32 {
    data.get("home.map-scale")
        .and_then(Value::as_f64)
        .map(|scale| scale as f32)
        .unwrap_or(1.)
        .clamp(0.5, 3.)
}

fn service_map_pan(data: &HashMap<String, Value>) -> (f32, f32) {
    (
        data.get("home.map-pan-x")
            .and_then(Value::as_f64)
            .unwrap_or(0.) as f32,
        data.get("home.map-pan-y")
            .and_then(Value::as_f64)
            .unwrap_or(0.) as f32,
    )
}

fn project_map_point(point: (f32, f32), scale: f32, pan: (f32, f32)) -> (f32, f32) {
    (
        0.5 + (point.0 - 0.5) * scale + pan.0,
        0.5 + (point.1 - 0.5) * scale + pan.1,
    )
}

fn service_map_positions(services: &[Value], scale: f32, pan: (f32, f32)) -> Vec<(f32, f32)> {
    let rows = services.len().div_ceil(2).max(1);
    services
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let x = if index % 2 == 0 { 0.25 } else { 0.75 };
            let y = (index / 2) as f32 + 0.5;
            project_map_point((x, y / rows as f32), scale, pan)
        })
        .collect()
}

fn point_segment_distance(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared <= f32::EPSILON {
        0.
    } else {
        (((point.0 - start.0) * dx + (point.1 - start.1) * dy) / length_squared).clamp(0., 1.)
    };
    let closest = (start.0 + t * dx, start.1 + t * dy);
    ((point.0 - closest.0).powi(2) + (point.1 - closest.1).powi(2)).sqrt()
}

fn hit_test_connection(
    edges: &[(Value, (f32, f32), (f32, f32))],
    point: (f32, f32),
    tolerance: f32,
) -> Option<Value> {
    edges
        .iter()
        .map(|(edge, start, end)| (point_segment_distance(point, *start, *end), edge))
        .filter(|(distance, _)| *distance <= tolerance)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, edge)| edge.clone())
}

#[derive(Clone)]
pub(crate) struct ServiceMapRoute {
    pub(crate) reference: Value,
    pub(crate) points: [(f32, f32); 4],
    pub(crate) weight: u64,
}

pub(crate) fn service_map_connections(data: &HashMap<String, Value>) -> Vec<Value> {
    let mut rows = Vec::new();
    for (key, result) in data {
        if key.starts_with("home.connections.") {
            rows.extend(discovery_rows(result));
        }
    }
    rows.sort_by(|a, b| {
        let a_key = (
            service_identity(&a["source"]),
            service_identity(&a["target"]),
        );
        let b_key = (
            service_identity(&b["source"]),
            service_identity(&b["target"]),
        );
        a_key.cmp(&b_key)
    });
    rows.dedup_by(|a, b| {
        service_identity(&a["source"]) == service_identity(&b["source"])
            && service_identity(&a["target"]) == service_identity(&b["target"])
    });
    rows
}

fn service_edge_route(
    source: (f32, f32),
    target: (f32, f32),
    source_left: bool,
    target_left: bool,
    lane: usize,
    gutter_center: f32,
    width: f32,
    height: f32,
) -> [(f32, f32); 4] {
    let source_x = source.0 * width
        + if source_left {
            width * 0.21
        } else {
            -width * 0.21
        };
    let target_x = target.0 * width
        + if target_left {
            width * 0.21
        } else {
            -width * 0.21
        };
    let lane_sign = if lane % 2 == 0 { -1. } else { 1. };
    let lane_distance = ((lane / 2 + 1) as f32 * 4.).min(width * 0.035);
    let lane_x = gutter_center + lane_sign * lane_distance;
    let source_y = source.1 * height;
    let target_y = target.1 * height;
    [
        (source_x, source_y),
        (lane_x, source_y),
        (lane_x, target_y),
        (target_x, target_y),
    ]
}

pub(crate) fn service_map_routes(
    services: &[Value],
    connections: &[Value],
    scale: f32,
    pan: (f32, f32),
    width: f32,
    height: f32,
) -> Vec<ServiceMapRoute> {
    let gutter_center = project_map_point((0.5, 0.5), scale, pan).0 * width;
    let positions = service_map_positions(services, scale, pan);
    connections
        .iter()
        .enumerate()
        .filter_map(|(lane, edge)| {
            let source_id = service_identity(&edge["source"]);
            let target_id = service_identity(&edge["target"]);
            let source = services
                .iter()
                .position(|service| service_identity(service) == source_id)?;
            let target = services
                .iter()
                .position(|service| service_identity(service) == target_id)?;
            Some(ServiceMapRoute {
                reference: edge.clone(),
                points: service_edge_route(
                    positions[source],
                    positions[target],
                    source % 2 == 0,
                    target % 2 == 0,
                    lane,
                    gutter_center,
                    width,
                    height,
                ),
                weight: edge["spanCount"].as_u64().unwrap_or(1),
            })
        })
        .collect()
}

fn hit_test_service_map_edges(
    edges: &[ServiceMapRoute],
    point: (f32, f32),
    tolerance: f32,
) -> Option<Value> {
    edges
        .iter()
        .filter_map(|edge| {
            let distance = edge
                .points
                .windows(2)
                .map(|segment| point_segment_distance(point, segment[0], segment[1]))
                .fold(f32::INFINITY, f32::min);
            (distance <= tolerance).then_some((distance, edge.reference.clone()))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, edge)| edge)
}

fn human_bytes(value: &Value) -> String {
    let bytes = ns(value).max(0) as f64;
    let (amount, unit) = if bytes >= (1_u64 << 30) as f64 {
        (bytes / (1_u64 << 30) as f64, "GiB")
    } else if bytes >= (1_u64 << 20) as f64 {
        (bytes / (1_u64 << 20) as f64, "MiB")
    } else if bytes >= (1_u64 << 10) as f64 {
        (bytes / (1_u64 << 10) as f64, "KiB")
    } else {
        (bytes, "B")
    };
    format!("{amount:.1} {unit}")
}

fn sorted_records(
    mut records: Vec<Value>,
    signal: &str,
    sort: &str,
    direction: &str,
) -> Vec<Value> {
    records.sort_by(|a, b| {
        let order = match sort {
            "name" => record_title(a, signal)
                .to_lowercase()
                .cmp(&record_title(b, signal).to_lowercase()),
            "count" => {
                let key = if signal == "traces" {
                    "spanCount"
                } else if signal == "metrics" {
                    "dataPointCount"
                } else {
                    "severityNumber"
                };
                a[key].as_u64().cmp(&b[key].as_u64())
            }
            _ => {
                let key = if signal == "traces" {
                    "startTime"
                } else if signal == "logs" {
                    "timestamp"
                } else {
                    "lastSeen"
                };
                a[key].as_str().cmp(&b[key].as_str())
            }
        };
        if direction == "asc" {
            order
        } else {
            order.reverse()
        }
    });
    records
}

impl Viewer {
    pub fn refresh_home(&mut self, cx: &mut Context<Self>) {
        let stale_pages: Vec<String> = self
            .generations
            .keys()
            .filter(|key| key.starts_with("home.services.more."))
            .cloned()
            .collect();
        for key in stale_pages {
            *self.generations.entry(key.clone()).or_default() += 1;
            self.data.remove(&key);
        }
        let stale_connections: Vec<String> = self
            .generations
            .keys()
            .filter(|key| key.starts_with("home.connections."))
            .cloned()
            .collect();
        for key in stale_connections {
            *self.generations.entry(key.clone()).or_default() += 1;
            self.data.remove(&key);
        }
        self.data.retain(|key, _| {
            !key.starts_with("home.connections.") && !key.starts_with("home.connection-offset.")
        });
        let input = self.search.read(cx).value().to_string();
        let (queries, invalid) = home_queries(&input);
        if let Some(message) = invalid {
            self.status = format!("Home search cannot match any signal: {message}");
        }
        let term = self.home_filter.read(cx).value().to_string();
        let options = json!({
            "kind":"services",
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "limit":100,
            "offset":0,
            "term":term,
            "queries":queries
        });
        self.rpc_key(
            "home.services",
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );

        if let (Some((name, namespace)), Some(identity)) =
            (self.selected_service.clone(), selected_identity(&self.data))
        {
            self.refresh_selected_service(name, namespace, identity, cx);
        }
    }

    fn load_more_home_services(&mut self, cx: &mut Context<Self>) {
        if !home_services_have_more(&self.data) {
            return;
        }
        let offset = next_home_services_offset(&self.data);
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let term = self.home_filter.read(cx).value().to_string();
        let options = json!({
            "kind":"services",
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "limit":100,
            "offset":offset,
            "term":term,
            "queries":queries
        });
        self.rpc_key(
            &format!("home.services.more.{offset}"),
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );
    }

    /// Called by the app when a newly returned services result is stored.
    /// Connections are source-service scoped by the backend contract, so query
    /// each typed identity independently and merge results while rendering.
    pub fn refresh_home_connections(&mut self, cx: &mut Context<Self>) {
        let services = home_service_rows(&self.data);
        if let Some(selected) = selected_identity(&self.data) {
            if !services
                .iter()
                .any(|service| service_identity(service) == selected)
            {
                self.selected_service = None;
                self.data.remove("home.selected-identity");
                self.data.remove("home.selected-edge");
            }
        }
        if self.selected_service.is_none() {
            if let Some(service) = services.first() {
                let name = field(service, "serviceName");
                let namespace = field(service, "serviceNamespace");
                let identity = service_identity(service);
                self.selected_service = Some((name.clone(), namespace.clone()));
                self.data
                    .insert("home.selected-identity".into(), json!(identity.clone()));
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let term = self.home_filter.read(cx).value().to_string();
        for service in &services {
            let identity = service_identity(service);
            if identity.is_empty()
                || self
                    .data
                    .contains_key(&format!("home.connection-offset.{identity}"))
            {
                continue;
            }
            self.data
                .insert(format!("home.connection-offset.{identity}"), json!(0));
            let options = json!({
                "kind":"connections",
                "serviceIdentity":identity,
                "startTime":self.time_range.start,
                "endTime":self.time_range.end,
                "term":term,
                "queries":queries,
                "limit":100,
                "offset":0
            });
            self.rpc_key(
                &format!("home.connections.{identity}.0"),
                "discoverTelemetry",
                json!({"options":options}),
                cx,
            );
        }
    }

    fn refresh_selected_service(
        &mut self,
        _name: String,
        _namespace: String,
        identity: String,
        cx: &mut Context<Self>,
    ) {
        self.data.retain(|key, _| {
            !key.starts_with("home.expanded-override.")
                && !key.starts_with("home.expanded-values.")
                && !key.starts_with("home.expanded-offset.")
        });
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let term = self.home_filter.read(cx).value().to_string();
        let (signal, owner) = owner_and_signal(&self.data);
        let options = json!({
            "kind":"keys",
            "signal":signal,
            "ownerType":owner,
            "serviceIdentity":identity,
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "term":term,
            "queries":queries,
            "sortBy":state_text(&self.data, &format!("home.keys-sort.{signal}"), "name"),
            "sortDirection":state_text(&self.data, &format!("home.keys-direction.{signal}"), "asc"),
            "limit":page_size(&self.data, &format!("keys.{signal}")),
            "offset":page_offset(&self.data, &format!("keys.{signal}"))
        });
        self.rpc_key(
            "home.keys",
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );

        if self.data.get("home.selected-key").is_some() {
            self.refresh_selected_values(cx);
        }

        let scope = self.data.get("home.telemetry-scope").cloned();
        let input = self.search.read(cx).value().to_string();
        let (search_queries, _) = home_queries(&input);
        let params = json!({
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "limit":500
        });
        for (key, method, signal) in [
            ("home.traces", "searchTraceSummaries", "traces"),
            ("home.logs", "searchLogSummaries", "logs"),
            ("home.metrics", "searchMetricSummaries", "metrics"),
        ] {
            if search_queries.get(signal) == Some(&Value::Bool(false)) {
                *self.generations.entry(key.to_owned()).or_default() += 1;
                self.data.insert(key.to_owned(), json!([]));
                continue;
            }
            let signal_query =
                telemetry_query(&identity, scope.as_ref(), search_queries.get(signal));
            let mut params = params.clone();
            params["query"] = signal_query;
            let sort = state_text(&self.data, &format!("home.sort.{signal}"), "time");
            let direction =
                state_text(&self.data, &format!("home.sort-direction.{signal}"), "desc");
            let sort_field = match (signal, sort.as_str()) {
                ("traces", "name") => "rootSpanName",
                ("traces", "count") => "spanCount",
                ("logs", "name") => "body",
                ("logs", "count") => "severity",
                ("metrics", "name") => "name",
                ("metrics", "count") => "dataPointCount",
                ("traces", _) => "startTime",
                ("logs", _) => "timestamp",
                _ => "lastSeen",
            };
            params["sort"] = json!({"field":sort_field,"direction":direction});
            self.rpc_key(key, method, params, cx);
        }
    }

    fn refresh_selected_values(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.data.get("home.selected-key").cloned() else {
            self.data.remove("home.values");
            return;
        };
        let Some(identity) = selected_identity(&self.data) else {
            return;
        };
        let (signal, owner) = owner_and_signal(&self.data);
        let term = self.home_filter.read(cx).value().to_string();
        let sort_by = state_text(&self.data, "home.values-sort", "frequency");
        let sort_direction = state_text(&self.data, "home.values-direction", "desc");
        let options = json!({
            "kind":"values",
            "signal":signal,
            "ownerType":owner,
            "serviceIdentity":identity,
            "key":key["key"],
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "term":term,
            "queries":home_queries(&self.search.read(cx).value().to_string()).0,
            "sortBy":sort_by,
            "sortDirection":sort_direction,
            "limit":page_size(&self.data, &format!("values.{signal}")),
            "offset":page_offset(&self.data, &format!("values.{signal}"))
        });
        self.rpc_key(
            "home.values",
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );
    }

    pub fn select_service(
        &mut self,
        name: String,
        namespace: String,
        identity: String,
        cx: &mut Context<Self>,
    ) {
        let changed = selected_identity(&self.data).as_deref() != Some(identity.as_str());
        self.data.remove("home.selected-edge");
        self.data.remove("home.connection-spans");
        self.data
            .insert("home.details-tab".into(), json!("telemetry"));
        self.selected_service = Some((name.clone(), namespace.clone()));
        self.data
            .insert("home.selected-identity".into(), json!(identity));
        if changed {
            self.data.remove("home.selected-key");
            self.data.remove("home.values");
            self.data.remove("home.telemetry-scope");
            for signal in SIGNALS {
                self.data
                    .insert(format!("home.offset.keys.{signal}"), json!(0));
                self.data
                    .insert(format!("home.offset.values.{signal}"), json!(0));
            }
        }
        for signal in SIGNALS {
            self.data.insert(format!("home.offset.{signal}"), json!(0));
        }
        self.refresh_selected_service(
            name,
            namespace,
            selected_identity(&self.data).unwrap_or_default(),
            cx,
        );
        cx.notify();
    }

    fn change_discovery_signal(&mut self, signal: &str, cx: &mut Context<Self>) {
        self.data
            .insert("home.discovery-signal".into(), json!(signal));
        self.data.remove("home.selected-key");
        self.data.remove("home.values");
        self.data.insert("home.keys-offset".into(), json!(0));
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn load_attribute_preview(&mut self, key: &Value, offset: usize, cx: &mut Context<Self>) {
        let Some(identity) = selected_identity(&self.data) else {
            return;
        };
        let (signal, owner) = owner_and_signal(&self.data);
        let id = attribute_entry_id(&identity, &signal, &owner, key);
        self.data
            .insert(format!("home.expanded-offset.{id}"), json!(offset));
        let term = self.home_filter.read(cx).value().to_string();
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let options = json!({
            "kind":"values",
            "signal":signal,
            "ownerType":owner,
            "serviceIdentity":identity,
            "key":key["key"],
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "term":term,
            "queries":queries,
            "sortBy":"frequency",
            "sortDirection":"desc",
            "limit":25,
            "offset":offset
        });
        self.rpc_key(
            &format!("home.expanded-values.{id}.{offset}"),
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );
    }

    fn set_attribute_expanded(&mut self, key: &Value, expanded: bool, cx: &mut Context<Self>) {
        let Some(identity) = selected_identity(&self.data) else {
            return;
        };
        let (signal, owner) = owner_and_signal(&self.data);
        let id = attribute_entry_id(&identity, &signal, &owner, key);
        self.data
            .insert(format!("home.expanded-override.{id}"), json!(expanded));
        if expanded
            && !self
                .data
                .contains_key(&format!("home.expanded-values.{id}.0"))
        {
            self.load_attribute_preview(key, 0, cx);
        }
        cx.notify();
    }

    fn set_all_attributes_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        let identity = selected_identity(&self.data).unwrap_or_default();
        let (signal, owner) = owner_and_signal(&self.data);
        let keys = discovery_rows(self.data.get("home.keys").unwrap_or(&Value::Null));
        if expanded {
            for key in keys {
                let id = attribute_entry_id(&identity, &signal, &owner, &key);
                self.data
                    .insert(format!("home.expanded-override.{id}"), json!(true));
                if !self
                    .data
                    .contains_key(&format!("home.expanded-values.{id}.0"))
                {
                    self.load_attribute_preview(&key, 0, cx);
                }
            }
        } else {
            self.data
                .retain(|key, _| !key.starts_with("home.expanded-override."));
        }
        cx.notify();
    }

    fn cycle_discovery_owner(&mut self, cx: &mut Context<Self>) {
        let (signal, owner) = owner_and_signal(&self.data);
        let owners = owner_options(&signal);
        let index = owners.iter().position(|value| *value == owner).unwrap_or(0);
        let next = owners[(index + 1) % owners.len()];
        self.data
            .insert(format!("home.discovery-owner.{signal}"), json!(next));
        self.data.remove("home.selected-key");
        self.data.remove("home.values");
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn choose_attribute_key(&mut self, key: Value, cx: &mut Context<Self>) {
        let changed = self.data.get("home.selected-key") != Some(&key);
        self.data.insert("home.selected-key".into(), key);
        if changed {
            let (signal, _) = owner_and_signal(&self.data);
            self.data
                .insert(format!("home.offset.values.{signal}"), json!(0));
        }
        self.data.remove("home.telemetry-scope");
        self.refresh_selected_values(cx);
        cx.notify();
    }

    fn scope_telemetry_to_value(&mut self, value: Value, cx: &mut Context<Self>) {
        let Some(key) = self.data.get("home.selected-key").cloned() else {
            return;
        };
        self.scope_telemetry_to_attribute_value(&key, value, cx);
    }

    fn scope_telemetry_to_attribute_value(
        &mut self,
        key: &Value,
        value: Value,
        cx: &mut Context<Self>,
    ) {
        let (signal, owner_type) = owner_and_signal(&self.data);
        self.data.insert(
            "home.telemetry-scope".into(),
            json!({
                "signal":signal,
                "ownerType":owner_type,
                "key":key["key"],
                "kind":key["kind"],
                "value":value
            }),
        );
        for signal in SIGNALS {
            self.data.insert(format!("home.offset.{signal}"), json!(0));
        }
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn set_discovery_page_size(
        &mut self,
        kind: &str,
        signal: &str,
        size: usize,
        cx: &mut Context<Self>,
    ) {
        let state = format!("{kind}.{signal}");
        self.data
            .insert(format!("home.page-size.{state}"), json!(size));
        self.data.insert(format!("home.offset.{state}"), json!(0));
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn set_discovery_offset(
        &mut self,
        kind: &str,
        signal: &str,
        offset: usize,
        cx: &mut Context<Self>,
    ) {
        let state = format!("{kind}.{signal}");
        self.data
            .insert(format!("home.offset.{state}"), json!(offset));
        if kind == "values" {
            self.refresh_selected_values(cx);
        } else if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn set_home_page_size(&mut self, signal: &str, size: usize, cx: &mut Context<Self>) {
        self.data
            .insert(format!("home.page-size.{signal}"), json!(size));
        self.data.insert(format!("home.offset.{signal}"), json!(0));
        cx.notify();
    }

    fn set_home_offset(&mut self, signal: &str, offset: usize, cx: &mut Context<Self>) {
        self.data
            .insert(format!("home.offset.{signal}"), json!(offset));
        cx.notify();
    }

    fn cycle_service_sort(&mut self, cx: &mut Context<Self>) {
        let current = state_text(&self.data, "home.service-sort", "name");
        let next = match current.as_str() {
            "name" => "traces",
            "traces" => "logs",
            "logs" => "metrics",
            "metrics" => "lastSeen",
            _ => "name",
        };
        self.data.insert("home.service-sort".into(), json!(next));
        let direction = state_text(&self.data, "home.service-direction", "asc");
        self.data.insert(
            "home.service-direction".into(),
            json!(if next == "name" || direction == "desc" {
                "asc"
            } else {
                "desc"
            }),
        );
        cx.notify();
    }

    fn cycle_key_sort(&mut self, signal: &str, cx: &mut Context<Self>) {
        let sort_key = format!("home.keys-sort.{signal}");
        let direction_key = format!("home.keys-direction.{signal}");
        let sort = state_text(&self.data, &sort_key, "name");
        let direction = state_text(&self.data, &direction_key, "asc");
        let (next_sort, next_direction) = match sort.as_str() {
            "name" if direction == "asc" => ("name", "desc"),
            "name" => ("count", "desc"),
            "count" if direction == "desc" => ("count", "asc"),
            "count" => ("frequency", "desc"),
            "frequency" if direction == "desc" => ("frequency", "asc"),
            _ => ("name", "asc"),
        };
        self.data.insert(sort_key, json!(next_sort));
        self.data.insert(direction_key, json!(next_direction));
        self.data
            .insert(format!("home.offset.keys.{signal}"), json!(0));
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn cycle_value_sort(&mut self, cx: &mut Context<Self>) {
        let sort = state_text(&self.data, "home.values-sort", "frequency");
        let direction = state_text(&self.data, "home.values-direction", "desc");
        if sort == "frequency" {
            self.data.insert("home.values-sort".into(), json!("name"));
            self.data
                .insert("home.values-direction".into(), json!("asc"));
        } else if direction == "asc" {
            self.data
                .insert("home.values-direction".into(), json!("desc"));
        } else {
            self.data
                .insert("home.values-sort".into(), json!("frequency"));
            self.data
                .insert("home.values-direction".into(), json!("desc"));
        }
        self.refresh_selected_values(cx);
        cx.notify();
    }

    fn cycle_telemetry_sort(&mut self, signal: &str, cx: &mut Context<Self>) {
        let sort = state_text(&self.data, &format!("home.sort.{signal}"), "time");
        let direction = state_text(&self.data, &format!("home.sort-direction.{signal}"), "desc");
        if sort == "time" {
            self.data
                .insert(format!("home.sort.{signal}"), json!("name"));
            self.data
                .insert(format!("home.sort-direction.{signal}"), json!("asc"));
        } else if sort == "name" {
            self.data
                .insert(format!("home.sort.{signal}"), json!("count"));
            self.data
                .insert(format!("home.sort-direction.{signal}"), json!("desc"));
        } else if direction == "desc" {
            self.data
                .insert(format!("home.sort-direction.{signal}"), json!("asc"));
        } else {
            self.data
                .insert(format!("home.sort.{signal}"), json!("time"));
            self.data
                .insert(format!("home.sort-direction.{signal}"), json!("desc"));
        }
        if let Some((name, namespace)) = self.selected_service.clone() {
            if let Some(identity) = selected_identity(&self.data) {
                self.refresh_selected_service(name, namespace, identity, cx);
            }
        }
        cx.notify();
    }

    fn select_connection(&mut self, edge: Value, cx: &mut Context<Self>) {
        self.data.insert("home.selected-edge".into(), edge.clone());
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let term = self.home_filter.read(cx).value().to_string();
        let options = json!({
            "kind":"connection-spans",
            "serviceIdentity":edge["source"]["serviceIdentity"],
            "targetIdentity":edge["target"]["serviceIdentity"],
            "startTime":self.time_range.start,
            "endTime":self.time_range.end,
            "term":term,
            "queries":queries,
            "limit":500
        });
        self.rpc_key(
            "home.connection-spans",
            "discoverTelemetry",
            json!({"options":options}),
            cx,
        );
        cx.notify();
    }

    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut services = home_service_rows(&self.data);
        let issue_signal = state_text(&self.data, "home.issue-signal", "");
        if !issue_signal.is_empty() {
            services.retain(|service| match issue_signal.as_str() {
                "traces" => service_count(service, "traces") > 0,
                "logs" => service_count(service, "logs") > 0,
                "metrics" => service_count(service, "metrics") > 0,
                _ => true,
            });
        }
        let selected_service_sort = state_text(&self.data, "home.service-sort", "name");
        let service_direction = state_text(&self.data, "home.service-direction", "asc");
        service_sort(&mut services, &selected_service_sort, &service_direction);
        let connections = self.connection_rows();
        let stats = self.data.get("getStats").cloned().unwrap_or(Value::Null);
        let list = self
            .prefs
            .list_open
            .then(|| self.render_home_welcome(&stats, cx));
        let main = self.render_service_map(&services, &connections, cx);
        let details = self
            .prefs
            .details_open
            .then(|| self.render_home_details(cx));
        self.render_workspace(list, main, details, window)
    }

    fn render_home_welcome(&mut self, stats: &Value, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let protocol = state_text(&self.data, "home.setup-protocol", "http");
        let backend = self.backend.as_ref();
        let endpoint = if protocol == "grpc" {
            backend.map(|backend| format!("http://127.0.0.1:{}", backend.config.grpc_port))
        } else {
            backend.map(|backend| format!("http://127.0.0.1:{}", backend.config.http_port))
        };
        let protocol_name = if protocol == "grpc" {
            "grpc"
        } else {
            "http/protobuf"
        };
        let endpoint_value = endpoint
            .clone()
            .unwrap_or_else(|| "Collector starting…".to_owned());
        let env = format!(
            "export OTEL_EXPORTER_OTLP_ENDPOINT=\\\n\"{}\"\nexport OTEL_EXPORTER_OTLP_PROTOCOL=\\\n\"{}\"\nexport OTEL_SERVICE_NAME=\"my-service\"",
            endpoint_value, protocol_name
        );
        let test_command = format!(
            "otel-cli exec \\\n  --endpoint {} \\\n  --protocol {} \\\n  --service my-service \\\n  --name \"test trace\" \\\n  -- sleep 1",
            endpoint.as_deref().unwrap_or("http://127.0.0.1:4318"),
            protocol_name
        );
        let escaped_env = env
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br>");
        let escaped_command = test_command
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\n', "<br>");
        let env_panel = div()
            .id("home-otlp-panel")
            .rounded_md()
            .border_1()
            .border_color(color(p.chrome))
            .overflow_hidden()
            .child(
                div()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .bg(color(p.panel))
                    .child(
                        widgets::button("home-otlp-http")
                            .label("HTTP")
                            .selected(protocol == "http")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data
                                    .insert("home.setup-protocol".into(), json!("http"));
                                cx.notify();
                            })),
                    )
                    .child(
                        widgets::button("home-otlp-grpc")
                            .label("gRPC")
                            .selected(protocol == "grpc")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data
                                    .insert("home.setup-protocol".into(), json!("grpc"));
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        widgets::button("home-copy-otlp-env")
                            .label("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(env.clone()))
                            }),
                    ),
            )
            .child(
                div()
                    .id("home-otlp-env")
                    .test_support()
                    .p_3()
                    .bg(color(p.canvas))
                    .child(
                        gpui_kit::component::text::TextView::html(
                            "home-otlp-env-text",
                            format!("<pre>{escaped_env}</pre>"),
                        )
                        .selectable(true),
                    ),
            );
        let cli_panel = div()
            .id("home-otel-cli-panel")
            .rounded_md()
            .border_1()
            .border_color(color(p.chrome))
            .overflow_hidden()
            .child(
                div()
                    .h(px(48.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .bg(color(p.panel))
                    .child("Shell")
                    .child(
                        widgets::button("home-copy-otel-cli")
                            .label("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    test_command.clone(),
                                ))
                            }),
                    ),
            )
            .child(
                div().p_3().bg(color(p.canvas)).child(
                    gpui_kit::component::text::TextView::html(
                        "home-otel-cli-text",
                        format!("<pre>{escaped_command}</pre>"),
                    )
                    .selectable(true),
                ),
            );
        let mut setup = div()
            .id("home-onboarding-content")
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(widgets::heading("Connect an exporter", p))
            .child(div().text_sm().text_color(color(p.subtle)).child(
                "Set these variables in the terminal where you start your instrumented app.",
            ))
            .child(env_panel)
            .child(widgets::heading("Send a test trace", p))
            .child(
                div()
                    .text_sm()
                    .text_color(color(p.subtle))
                    .child("Run this with otel-cli installed."),
            )
            .child(cli_panel);
        if endpoint.is_none() {
            setup = setup.child(widgets::empty(
                "Receiver addresses will appear when the Collector is ready.",
                p,
            ));
        }
        let sidebar = div()
            .id("home-onboarding")
            .test_support()
            .role(Role::Group)
            .aria_label("OTel Desktop Viewer onboarding")
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .py_2()
                    .child(self.lulu_mascot())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .text_lg()
                            .child("OTel Desktop Viewer")
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(color(p.subtle))
                                    .child("Now with “a lot more functionality”™"),
                            ),
                    ),
            )
            .child(
                div()
                    .id("home-onboarding-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .id("home-import-dropzone")
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Import telemetry JSON files")
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_h(px(96.))
                            .m_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(color(p.chrome))
                            .cursor_pointer()
                            .child(
                                widgets::button("home-import")
                                    .label("Drop telemetry JSON files here or Import…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_files(window, cx)
                                    })),
                            ),
                    )
                    .child(setup),
            )
            .child(self.storage_and_issues(stats, cx));
        // The shared workspace assigns the actual pane width and dividers.
        // Keep this panel's contents fluid so the right edge remains draggable.
        sidebar.into_any_element()
    }

    fn lulu_mascot(&self) -> AnyElement {
        let sprite = include_str!(
            "../../desktopexporter/internal/frontend/src/assets/images/lulu-pixel.txt"
        );
        let rows: Vec<Vec<char>> = sprite.lines().map(|row| row.chars().collect()).collect();
        let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as f32;
        let height = rows.len().max(1) as f32;
        div()
            .id("home-lulu-mascot")
            .test_support()
            .role(Role::Image)
            .aria_label("Lulu the axolotl")
            .w(px(52.))
            .h(px(44.))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let cell_w = bounds.size.width / width;
                        let cell_h = bounds.size.height / height;
                        for (y, row) in rows.iter().enumerate() {
                            for (x, pixel) in row.iter().enumerate() {
                                let ink = match pixel {
                                    'o' | 'e' => color(0x9b334e),
                                    's' => color(0xe8adae),
                                    'b' => color(0xe6c0bd),
                                    'g' => color(0xed7d93),
                                    'G' => color(0xdb6b7c),
                                    'w' => color(0xffe5ed),
                                    _ => continue,
                                };
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: bounds.origin
                                            + point(cell_w * x as f32, cell_h * y as f32),
                                        size: size(cell_w, cell_h),
                                    },
                                    ink,
                                ));
                            }
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    fn render_home_details(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self
            .data
            .get("home.selected-edge")
            .unwrap_or(&Value::Null)
            .is_null()
        {
            return self.render_connection_details(cx);
        }
        let p = palette(self.prefs.dark);
        let Some((name, namespace)) = self.selected_service.clone() else {
            return div()
                .id("home-details-empty")
                .h_full()
                .child(widgets::empty(
                    "Select a service or connection to inspect telemetry.",
                    p,
                ))
                .into_any_element();
        };
        let active = state_text(&self.data, "home.details-tab", "telemetry");
        let service = home_service_rows(&self.data)
            .into_iter()
            .find(|service| {
                service_identity(service) == selected_identity(&self.data).unwrap_or_default()
            })
            .unwrap_or_else(|| json!({"serviceName":name,"serviceNamespace":namespace}));
        let mut header = div()
            .id("home-details-header")
            .flex()
            .items_center()
            .justify_between()
            .gap_1()
            .h(px(48.))
            .px_2()
            .mb_2()
            .rounded_lg()
            .bg(color(p.panel))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_ellipsis()
                    .child(service_label(&service)),
            );
        for (tab, label) in [("telemetry", "Telemetry"), ("attributes", "Attributes")] {
            let selected = active == tab;
            header = header.child(
                widgets::button(format!("home-details-{tab}"))
                    .label(label)
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.data.insert("home.details-tab".into(), json!(tab));
                        cx.notify();
                    })),
            );
        }
        let content = if active == "attributes" {
            self.render_service_attributes(&name, &namespace, cx)
        } else {
            self.render_service_telemetry(cx)
        };
        div()
            .id("home-details-panel")
            .test_support()
            .role(Role::Group)
            .aria_label("Selected service details")
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(header)
            .child(content)
            .into_any_element()
    }

    fn has_more_connections(&self) -> bool {
        let services = home_service_rows(&self.data);
        services.iter().any(|service| {
            let identity = service_identity(service);
            let offset = self
                .data
                .get(&format!("home.connection-offset.{identity}"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            self.data
                .get(&format!("home.connections.{identity}.{offset}"))
                .is_some_and(|page| page["truncated"] == true)
        })
    }

    fn load_more_connections(&mut self, cx: &mut Context<Self>) {
        let services = home_service_rows(&self.data);
        let input = self.search.read(cx).value().to_string();
        let (queries, _) = home_queries(&input);
        let term = self.home_filter.read(cx).value().to_string();
        for service in &services {
            let identity = service_identity(service);
            let offset_key = format!("home.connection-offset.{identity}");
            let offset = self
                .data
                .get(&offset_key)
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let previous_key = format!("home.connections.{identity}.{offset}");
            if !self
                .data
                .get(&previous_key)
                .is_some_and(|page| page["truncated"] == true)
            {
                continue;
            }
            let next = offset + 100;
            self.data.insert(offset_key, json!(next));
            let options = json!({
                "kind":"connections", "serviceIdentity":identity,
                "startTime":self.time_range.start, "endTime":self.time_range.end,
                "term":term, "queries":queries, "limit":100, "offset":next
            });
            self.rpc_key(
                &format!("home.connections.{identity}.{next}"),
                "discoverTelemetry",
                json!({"options":options}),
                cx,
            );
        }
    }

    fn change_service_map_view(&mut self, action: &str, cx: &mut Context<Self>) {
        let scale = service_map_scale(&self.data);
        let (pan_x, pan_y) = service_map_pan(&self.data);
        match action {
            "zoom-in" => {
                self.data
                    .insert("home.map-scale".into(), json!((scale + 0.25).min(3.)));
            }
            "zoom-out" => {
                self.data
                    .insert("home.map-scale".into(), json!((scale - 0.25).max(0.5)));
            }
            "fit" => {
                self.data.insert("home.map-scale".into(), json!(1.));
                self.data.insert("home.map-pan-x".into(), json!(0.));
                self.data.insert("home.map-pan-y".into(), json!(0.));
            }
            "left" => {
                self.data.insert(
                    "home.map-pan-x".into(),
                    json!((pan_x + 0.08).clamp(-0.45, 0.45)),
                );
            }
            "right" => {
                self.data.insert(
                    "home.map-pan-x".into(),
                    json!((pan_x - 0.08).clamp(-0.45, 0.45)),
                );
            }
            "up" => {
                self.data.insert(
                    "home.map-pan-y".into(),
                    json!((pan_y + 0.08).clamp(-0.45, 0.45)),
                );
            }
            "down" => {
                self.data.insert(
                    "home.map-pan-y".into(),
                    json!((pan_y - 0.08).clamp(-0.45, 0.45)),
                );
            }
            _ => return,
        }
        cx.notify();
    }

    fn connection_rows(&self) -> Vec<Value> {
        service_map_connections(&self.data)
    }

    fn storage_and_issues(&mut self, stats: &Value, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let storage = &stats["storage"];
        let used = storage["sizeBytes"].as_u64().unwrap_or(0);
        let cap = storage["maxSizeBytes"].as_u64().unwrap_or(0);
        let percent = if cap == 0 {
            0.
        } else {
            (used as f64 / cap as f64).clamp(0., 1.)
        };
        let storage_open = state_bool(&self.data, "home.storage-open", false);
        let issues_open = state_bool(&self.data, "home.ingestion-open", false);
        let rejections = stats["rejections"].as_array().cloned().unwrap_or_default();
        let mut section = div()
            .id("home-storage-issues")
            .flex()
            .flex_col()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(color(p.chrome))
            .child(
                widgets::button("home-storage-toggle")
                    .label("Storage")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.data.insert(
                            "home.storage-open".into(),
                            json!(!state_bool(&this.data, "home.storage-open", false)),
                        );
                        cx.notify();
                    })),
            );
        if storage_open {
            section = section.child(widgets::property(
                "Usage",
                if storage["inMemory"] == true {
                    format!("{} · in memory", human_bytes(&storage["sizeBytes"]))
                } else if cap == 0 {
                    format!("{} · no retention cap", human_bytes(&storage["sizeBytes"]))
                } else {
                    format!(
                        "{} / {}",
                        human_bytes(&storage["sizeBytes"]),
                        human_bytes(&storage["maxSizeBytes"])
                    )
                },
                p,
            ));
            if cap > 0 {
                section = section.child(
                    div().h(px(6.)).rounded_full().bg(color(p.chrome)).child(
                        div()
                            .w(relative(percent as f32))
                            .h_full()
                            .rounded_full()
                            .bg(color(p.accent)),
                    ),
                );
            }
        }
        section = section.child(
            widgets::button("home-ingestion-toggle")
                .label(if rejections.is_empty() {
                    "Ingestion issues · 0".to_owned()
                } else {
                    format!("Ingestion issues · {}", rejections.len())
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.data.insert(
                        "home.ingestion-open".into(),
                        json!(!state_bool(&this.data, "home.ingestion-open", false)),
                    );
                    cx.notify();
                })),
        );
        if !issues_open {
            return section.into_any_element();
        }
        if rejections.is_empty() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(color(p.subtle))
                    .child("No rejected telemetry has been reported."),
            );
        } else {
            for (index, issue) in rejections.iter().enumerate() {
                let signal = field(issue, "signal");
                let samples = issue["samples"].as_array().cloned().unwrap_or_default();
                let sample = samples.first().map(attribute).unwrap_or_default();
                let copy = sample.clone();
                section = section.child(
                    div()
                        .id(("ingestion-issue", index))
                        .p_2()
                        .rounded_md()
                        .bg(color(p.canvas))
                        .child(format!(
                            "{} · {} ×{}",
                            signal,
                            field(issue, "kind"),
                            field(issue, "occurrences")
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(color(p.subtle))
                                .child(format!("Last seen {}", field(issue, "lastSeen"))),
                        )
                        .child(div().text_xs().text_ellipsis().child(sample.clone()))
                        .child(
                            div()
                                .flex()
                                .gap_1()
                                .child(
                                    widgets::button(("issue-signal", index))
                                        .label(format!("View {signal} services"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.data
                                                .insert("home.issue-signal".into(), json!(signal));
                                            this.prefs.list_open = true;
                                            this.selected_service = None;
                                            this.data.remove("home.selected-identity");
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    widgets::button(("copy-issue", index))
                                        .label("Copy sample")
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                ),
                        ),
                );
            }
        }
        section.into_any_element()
    }

    fn render_service_map(
        &mut self,
        services: &[Value],
        connections: &[Value],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let selected_service_sort = state_text(&self.data, "home.service-sort", "name");
        let service_direction = state_text(&self.data, "home.service-direction", "asc");
        let issue_signal = state_text(&self.data, "home.issue-signal", "");
        let scale = service_map_scale(&self.data);
        let map_view =
            div()
                .id("service-map-view")
                .test_support()
                .role(Role::Group)
                .aria_label("Service map view")
                .flex()
                .items_center()
                .gap_0()
                .child(widgets::button("service-map-pan-left").label("←").on_click(
                    cx.listener(|this, _, _, cx| this.change_service_map_view("left", cx)),
                ))
                .child(
                    widgets::button("service-map-pan-up").label("↑").on_click(
                        cx.listener(|this, _, _, cx| this.change_service_map_view("up", cx)),
                    ),
                )
                .child(widgets::button("service-map-pan-down").label("↓").on_click(
                    cx.listener(|this, _, _, cx| this.change_service_map_view("down", cx)),
                ))
                .child(
                    widgets::button("service-map-pan-right")
                        .label("→")
                        .on_click(
                            cx.listener(|this, _, _, cx| this.change_service_map_view("right", cx)),
                        ),
                )
                .child(widgets::button("service-map-zoom-out").label("−").on_click(
                    cx.listener(|this, _, _, cx| this.change_service_map_view("zoom-out", cx)),
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(color(p.subtle))
                        .px_1()
                        .child(format!("{}%", (scale * 100.).round() as u32)),
                )
                .child(widgets::button("service-map-zoom-in").label("+").on_click(
                    cx.listener(|this, _, _, cx| this.change_service_map_view("zoom-in", cx)),
                ))
                .child(widgets::button("service-map-fit").label("Fit").on_click(
                    cx.listener(|this, _, _, cx| this.change_service_map_view("fit", cx)),
                ));
        let filter = div()
            .flex()
            .items_center()
            .gap_1()
            .flex_1()
            .min_w_0()
            .child(Input::new(&self.home_filter));
        let mut map = div()
            .id("home-service-map")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_2()
                    .h(px(48.))
                    .mb_2()
                    .rounded_lg()
                    .bg(color(p.panel))
                    .child(div().text_sm().child("Service map"))
                    .child(
                        div()
                            .id("home-service-count")
                            .text_xs()
                            .text_color(color(p.subtle))
                            .aria_label(format!(
                                "{} services · {} directed connections",
                                services.len(),
                                connections.len()
                            ))
                            .child(format!("{} / {}", services.len(), connections.len())),
                    )
                    .child(map_view),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .child(filter)
                    .child(
                        widgets::button("home-service-sort")
                            .label(format!("{} {}", selected_service_sort, service_direction))
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_service_sort(cx))),
                    )
                    .children((!issue_signal.is_empty()).then(|| {
                        widgets::button("clear-issue-filter")
                            .label(format!("{issue_signal} ×"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data.remove("home.issue-signal");
                                cx.notify();
                            }))
                    }))
                    .children(home_services_have_more(&self.data).then(|| {
                        widgets::button("home-more-services-map")
                            .label("More")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.load_more_home_services(cx)),
                            )
                    }))
                    .children(self.has_more_connections().then(|| {
                        widgets::button("more-service-connections")
                            .label("More connections")
                            .on_click(cx.listener(|this, _, _, cx| this.load_more_connections(cx)))
                    })),
            );
        if services.is_empty() {
            let stats = self.data.get("getStats").cloned().unwrap_or(Value::Null);
            let rejections = stats["rejections"].as_array().map_or(0, Vec::len);
            let mut getting_started = div().p_5().flex().flex_col().gap_2()
                .child(widgets::heading("Send OpenTelemetry data to get started", p))
                .child(widgets::empty("Import an OTLP JSON file, or configure an SDK to send telemetry to the local receivers.", p));
            if let Some(backend) = self.backend.as_ref() {
                let http_endpoint = format!("http://127.0.0.1:{}", backend.config.http_port);
                let grpc_endpoint = format!("127.0.0.1:{}", backend.config.grpc_port);
                let http_env = format!(
                    "export OTEL_EXPORTER_OTLP_ENDPOINT={http_endpoint}\nexport OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf"
                );
                let grpc_env = format!(
                    "export OTEL_EXPORTER_OTLP_ENDPOINT=http://{grpc_endpoint}\nexport OTEL_EXPORTER_OTLP_PROTOCOL=grpc"
                );
                getting_started = getting_started
                    .child(widgets::property(
                        "OTLP HTTP · protobuf",
                        http_endpoint.clone(),
                        p,
                    ))
                    .child(
                        widgets::button("copy-otlp-http-env")
                            .label("Copy HTTP SDK environment")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(http_env.clone()))
                            }),
                    )
                    .child(widgets::property("OTLP gRPC", grpc_endpoint.clone(), p))
                    .child(
                        widgets::button("copy-otlp-grpc-env")
                            .label("Copy gRPC SDK environment")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(grpc_env.clone()))
                            }),
                    );
            } else {
                getting_started = getting_started.child(widgets::empty(
                    "Receiver addresses are available once the Collector starts.",
                    p,
                ));
            }
            if rejections > 0 {
                getting_started = getting_started.child(widgets::property(
                    "Rejected telemetry",
                    rejections.to_string(),
                    p,
                ));
            }
            map = map.child(getting_started);
        } else {
            map = map.child(self.service_graph(services, connections, cx));
        }
        map.into_any_element()
    }

    fn service_graph(
        &self,
        services: &[Value],
        connections: &[Value],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let count = services.len();
        let scale = service_map_scale(&self.data);
        let pan = service_map_pan(&self.data);
        let rows = count.div_ceil(2).max(1);
        let graph_height = (rows as f32 * 64. + 24.).max(320.);
        let positions = service_map_positions(services, scale, pan);
        let canvas_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::new(Cell::new(None));
        let click_bounds = canvas_bounds.clone();
        let drag_bounds = canvas_bounds.clone();
        let route_services = services.to_vec();
        let route_connections = connections.to_vec();
        let click_services = route_services.clone();
        let click_connections = route_connections.clone();
        let click_scale = scale;
        let click_pan = pan;
        let mut graph = div()
            .id("service-map-canvas")
            .test_support()
            .role(Role::Group)
            .aria_label("Service map canvas; drag empty space to pan")
            .relative()
            .flex_1()
            .min_h(px(graph_height))
            .m_3()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    let Some(bounds) = drag_bounds.get() else {
                        return;
                    };
                    let (pan_x, pan_y) = service_map_pan(&this.data);
                    this.data.insert(
                        "home.map.drag".into(),
                        json!({
                            "x": f32::from(event.position.x),
                            "y": f32::from(event.position.y),
                            "canvasX": f32::from(bounds.origin.x),
                            "canvasY": f32::from(bounds.origin.y),
                            "canvasWidth": f32::from(bounds.size.width),
                            "canvasHeight": f32::from(bounds.size.height),
                            "panX": pan_x,
                            "panY": pan_y
                        }),
                    );
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some(drag) = this.data.get("home.map.drag").cloned() else {
                    return;
                };
                let width = drag["canvasWidth"].as_f64().unwrap_or(1.).max(1.) as f32;
                let height = drag["canvasHeight"].as_f64().unwrap_or(1.).max(1.) as f32;
                let x = f32::from(event.position.x);
                let y = f32::from(event.position.y);
                let pan_x = drag["panX"].as_f64().unwrap_or(0.) as f32
                    + (x - drag["x"].as_f64().unwrap_or(x as f64) as f32) / width;
                let pan_y = drag["panY"].as_f64().unwrap_or(0.) as f32
                    + (y - drag["y"].as_f64().unwrap_or(y as f64) as f32) / height;
                this.data
                    .insert("home.map-pan-x".into(), json!(pan_x.clamp(-0.45, 0.45)));
                this.data
                    .insert("home.map-pan-y".into(), json!(pan_y.clamp(-0.45, 0.45)));
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.data.remove("home.map.drag");
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !matches!(event, ClickEvent::Mouse(_)) {
                    return;
                }
                let Some(bounds) = click_bounds.get() else {
                    return;
                };
                if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                    return;
                }
                let position = event.position();
                let point = (
                    f32::from(position.x - bounds.origin.x),
                    f32::from(position.y - bounds.origin.y),
                );
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                let routes = service_map_routes(
                    &click_services,
                    &click_connections,
                    click_scale,
                    click_pan,
                    width,
                    height,
                );
                if let Some(edge) = hit_test_service_map_edges(&routes, point, 6.) {
                    this.select_connection(edge, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        canvas_bounds.set(Some(bounds));
                        let routes = service_map_routes(
                            &route_services,
                            &route_connections,
                            scale,
                            pan,
                            f32::from(bounds.size.width),
                            f32::from(bounds.size.height),
                        );
                        for route in &routes {
                            let mut path =
                                PathBuilder::stroke(px(if route.weight > 1 { 2. } else { 1.5 }));
                            path.move_to(
                                bounds.origin + point(px(route.points[0].0), px(route.points[0].1)),
                            );
                            path.line_to(
                                bounds.origin + point(px(route.points[1].0), px(route.points[1].1)),
                            );
                            path.line_to(
                                bounds.origin + point(px(route.points[2].0), px(route.points[2].1)),
                            );
                            let start = route.points[2];
                            let end = route.points[3];
                            let dx = end.0 - start.0;
                            let dy = end.1 - start.1;
                            let length = (dx * dx + dy * dy).sqrt().max(1.);
                            let ux = dx / length;
                            let uy = dy / length;
                            let tip = point(px(end.0 - 10. * ux), px(end.1 - 10. * uy));
                            let base = point(tip.x - px(9.) * ux, tip.y - px(9.) * uy);
                            let side = 4. * px(1.);
                            let left = point(base.x - side * uy, base.y + side * ux);
                            let right = point(base.x + side * uy, base.y - side * ux);
                            path.line_to(bounds.origin + tip);
                            path.move_to(bounds.origin + left);
                            path.line_to(bounds.origin + tip);
                            path.line_to(bounds.origin + right);
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color(p.subtle));
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        for (index, (service, (x, y))) in services.iter().zip(positions).enumerate() {
            let name = field(service, "serviceName");
            let namespace = field(service, "serviceNamespace");
            let identity = service_identity(service);
            let selected = self
                .data
                .get("home.selected-identity")
                .and_then(Value::as_str)
                == Some(identity.as_str());
            let node_color = service_color(&name, self.prefs.dark);
            let label = if name.is_empty() {
                "(unnamed)".to_owned()
            } else if field(service, "serviceNameKind") == "string" {
                name.clone()
            } else {
                format!("{} · {}", name, field(service, "serviceNameKind"))
            };
            let click_name = name.clone();
            let click_namespace = namespace.clone();
            let click_identity = identity.clone();
            graph = graph.child(
                div()
                    .id(("service-card", index))
                    .test_support()
                    .role(Role::Group)
                    .aria_label(label.clone())
                    .w(relative(0.42))
                    .absolute()
                    .left(relative(x - 0.21))
                    .top(relative(y))
                    .mt(px(-30.))
                    .child(
                        div()
                            .id(("graph-node", index))
                            .test_support()
                            .role(Role::Button)
                            .aria_label(label.clone())
                            .aria_selected(selected)
                            .flex()
                            .flex_col()
                            .gap_1()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .border_l_2()
                            .border_color(color(node_color))
                            .bg(color(if selected { p.selection } else { p.panel }))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_service(
                                    click_name.clone(),
                                    click_namespace.clone(),
                                    click_identity.clone(),
                                    cx,
                                );
                                cx.stop_propagation();
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(div().size(px(6.)).rounded_full().bg(color(node_color)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(px(11.))
                                            .text_ellipsis()
                                            .child(label),
                                    )
                                    .children(
                                        (service["errorSpanCount"].as_u64().unwrap_or(0) > 0).then(
                                            || {
                                                div()
                                                    .ml_auto()
                                                    .text_xs()
                                                    .text_color(color(p.error))
                                                    .child(format!(
                                                        "{} err",
                                                        service["errorSpanCount"]
                                                            .as_u64()
                                                            .unwrap_or(0)
                                                    ))
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_1()
                                    .text_size(px(9.))
                                    .text_color(color(p.subtle))
                                    .child(display_count(service, "traces"))
                                    .child("·")
                                    .child(display_count(service, "metrics"))
                                    .child("·")
                                    .child(display_count(service, "logs")),
                            )
                            .h(px(60.)),
                    ),
            );
        }
        graph.into_any_element()
    }

    fn render_connection_details(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let spans = discovery_rows(
            self.data
                .get("home.connection-spans")
                .unwrap_or(&Value::Null),
        );
        let edge = self
            .data
            .get("home.selected-edge")
            .cloned()
            .unwrap_or(Value::Null);
        let mut details = div()
            .id("home-details")
            .test_support()
            .role(Role::Group)
            .aria_label("Connection evidence")
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .child(widgets::heading("Connection evidence", p));
        if !edge.is_null() {
            details = details.child(widgets::property("From", service_label(&edge["source"]), p));
            details = details.child(widgets::property("To", service_label(&edge["target"]), p));
            details = details.child(widgets::property("Spans", field(&edge, "spanCount"), p));
            details = details.child(widgets::heading(
                format!("{} matching spans", spans.len()),
                p,
            ));
            for (index, span) in spans.iter().enumerate() {
                let trace_id = field(span, "traceID");
                details = details.child(
                    widgets::button(("connection-span", index))
                        .label(format!(
                            "{} · {}",
                            field(span, "name"),
                            field(span, "serviceName")
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_trace(trace_id.clone(), cx)
                        })),
                );
            }
            if spans.is_empty() {
                details = details.child(widgets::empty(
                    "No spans match this connection and current query.",
                    p,
                ));
            }
        } else {
            details = details.child(widgets::empty(
                "Select a directed connection to inspect its span evidence.",
                p,
            ));
        }
        details.into_any_element()
    }

    fn render_service_attributes(
        &mut self,
        name: &str,
        namespace: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette(self.prefs.dark);
        let (signal, owner) = owner_and_signal(&self.data);
        let identity = selected_identity(&self.data).unwrap_or_default();
        let selected_key = self
            .data
            .get("home.selected-key")
            .cloned()
            .unwrap_or(Value::Null);
        let key_response = self.data.get("home.keys").cloned().unwrap_or(Value::Null);
        let keys = discovery_rows(&key_response);
        let key_state = format!("keys.{signal}");
        let key_size = page_size(&self.data, &key_state);
        let key_offset = page_offset(&self.data, &key_state);
        let selected_service = discovery_rows(
            self.data.get("home.services").unwrap_or(&Value::Null),
        )
        .into_iter()
        .find(|service| service_identity(service) == identity)
        .unwrap_or_else(
            || json!({"serviceName":name,"serviceNamespace":namespace,"serviceIdentity":identity}),
        );
        let mut main = div()
            .id("home-attributes")
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .p_3()
                    .child(widgets::heading(
                        format!("Attributes · {}", service_label(&selected_service)),
                        p,
                    ))
                    .child(
                        widgets::button("service-map-back")
                            .label("Telemetry")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.data
                                    .insert("home.details-tab".into(), json!("telemetry"));
                                cx.notify();
                            })),
                    ),
            );

        let mut controls = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .px_3()
            .pb_2();
        for tab in SIGNALS {
            let selected = tab == signal;
            controls = controls.child(
                widgets::button(format!("attribute-signal-{tab}"))
                    .label(if selected {
                        format!("• {tab}")
                    } else {
                        tab.into()
                    })
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.change_discovery_signal(tab, cx)),
                    ),
            );
        }
        controls = controls
            .child(
                widgets::button("attribute-owner")
                    .label(format!("Owner: {owner} ▾"))
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_discovery_owner(cx))),
            )
            .child(
                widgets::button("attribute-sort")
                    .label(format!(
                        "Values: {} {}",
                        state_text(&self.data, "home.values-sort", "frequency"),
                        state_text(&self.data, "home.values-direction", "desc")
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.cycle_value_sort(cx))),
            );
        if self.data.get("home.telemetry-scope").is_some() {
            controls = controls.child(
                widgets::button("clear-attribute-scope")
                    .label("Clear telemetry scope")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.data.remove("home.telemetry-scope");
                        if let Some((name, namespace)) = this.selected_service.clone() {
                            if let Some(identity) = selected_identity(&this.data) {
                                this.refresh_selected_service(name, namespace, identity, cx);
                            }
                        }
                        cx.notify();
                    })),
            );
        }
        main = main.child(controls);

        let mut content = div().flex().flex_1().min_h_0().gap_2().px_3().pb_3();
        let mut key_list = div()
            .id("attribute-keys")
            .w(relative(0.42))
            .min_w(px(180.))
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(color(p.panel))
            .rounded_md()
            .child(widgets::heading(
                format!("{} keys · {owner} · {signal}", keys.len()),
                p,
            ))
            .child(
                widgets::button("home-keys-sort")
                    .label(format!(
                        "Sort: {} {}",
                        state_text(&self.data, &format!("home.keys-sort.{signal}"), "name"),
                        state_text(&self.data, &format!("home.keys-direction.{signal}"), "asc")
                    ))
                    .on_click(cx.listener({
                        let signal = signal.clone();
                        move |this, _, _, cx| this.cycle_key_sort(&signal, cx)
                    })),
            )
            .child(
                widgets::button("expand-all-attribute-values")
                    .label("Expand all on page")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_all_attributes_expanded(true, cx)),
                    ),
            )
            .child(
                widgets::button("collapse-all-attribute-values")
                    .label("Collapse all")
                    .on_click(
                        cx.listener(|this, _, _, cx| this.set_all_attributes_expanded(false, cx)),
                    ),
            );
        for (index, key) in keys.iter().skip(key_offset).take(key_size).enumerate() {
            let entry = key.clone();
            let identity_key = attribute_entry_id(&identity, &signal, &owner, key);
            let expanded = attribute_entry_expanded(&self.data, &identity_key);
            let selected = selected_key == *key;
            let key_name = field(key, "key");
            let key_kind = field(key, "kind");
            let copy = key_name.clone();
            key_list = key_list.child(
                div()
                    .id(("attribute-key", index))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(format!("{} ({})", key_name, key_kind))
                    .aria_selected(selected)
                    .flex()
                    .flex_col()
                    .items_stretch()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(color(p.chrome))
                    .bg(color(if selected { p.selection } else { p.panel }))
                    .cursor_pointer()
                    .on_click(cx.listener({
                        let selected_entry = entry.clone();
                        move |this, _, _, cx| this.choose_attribute_key(selected_entry.clone(), cx)
                    }))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().flex_1().min_w_0().text_ellipsis().child(key_name))
                            .child(div().text_xs().text_color(color(p.subtle)).child(key_kind)),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_1()
                            .child(div().text_xs().text_color(color(p.subtle)).child(format!(
                                "{} / {} · {:.0}%",
                                field(key, "count"),
                                field(key, "denominator"),
                                number(&key["relativeFrequency"]).unwrap_or(0.) * 100.
                            )))
                            .child(
                                widgets::button(("toggle-attribute-values", index))
                                    .label(if expanded { "Hide" } else { "Values" })
                                    .on_click(cx.listener({
                                        let key = entry.clone();
                                        move |this, _, _, cx| {
                                            this.set_attribute_expanded(&key, !expanded, cx)
                                        }
                                    })),
                            )
                            .child(
                                widgets::button(("copy-attribute-key", index))
                                    .label("Copy")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    }),
                            ),
                    ),
            );
            if expanded {
                let offset = self
                    .data
                    .get(&format!("home.expanded-offset.{identity_key}"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let preview = self
                    .data
                    .get(&format!("home.expanded-values.{identity_key}.{offset}"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let preview_rows = discovery_rows(&preview);
                let mut preview_list = div()
                    .id(("attribute-values-preview", index))
                    .test_support()
                    .role(Role::Group)
                    .aria_label(format!("{} values", field(key, "key")))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pl_5()
                    .pb_2();
                for (value_index, row) in preview_rows.iter().enumerate() {
                    let typed = row["value"].clone();
                    let copy = attribute(&typed);
                    let scope_key = entry.clone();
                    let scope_value = typed.clone();
                    preview_list = preview_list.child(
                        div()
                            .id(("attribute-preview-value", index * 100 + value_index))
                            .test_support()
                            .role(Role::ListItem)
                            .aria_label(format!(
                                "{} · {} of {} records",
                                copy,
                                field(row, "count"),
                                field(row, "denominator")
                            ))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().text_ellipsis().child(copy.clone()))
                            .child(format!("{} records", field(row, "count")))
                            .child(
                                widgets::button(("scope-preview-value", index * 100 + value_index))
                                    .label("Scope")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.scope_telemetry_to_attribute_value(
                                            &scope_key,
                                            scope_value.clone(),
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                widgets::button(("copy-preview-value", index * 100 + value_index))
                                    .label("Copy")
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    }),
                            ),
                    );
                }
                if preview.is_null() {
                    preview_list =
                        preview_list.child(widgets::empty("Loading attribute values…", p));
                }
                if preview["truncated"] == true {
                    let key = entry.clone();
                    let next_offset = offset + preview_rows.len();
                    preview_list = preview_list.child(
                        widgets::button(("more-preview-values", index))
                            .label("More values")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_attribute_preview(&key, next_offset, cx)
                            })),
                    );
                }
                key_list = key_list.child(preview_list);
            }
        }
        if keys.is_empty() {
            key_list = key_list.child(widgets::empty(
                "No attributes match this service, query, and filter.",
                p,
            ));
        }
        let key_signal_prev = signal.clone();
        let key_signal_next = signal.clone();
        let key_signal_size = signal.clone();
        let key_can_next = key_response["truncated"] == true;
        key_list = key_list.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .p_2()
                .child(
                    widgets::button("home-keys-prev")
                        .label("Previous")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let current =
                                page_offset(&this.data, &format!("keys.{key_signal_prev}"));
                            this.set_discovery_offset(
                                "keys",
                                &key_signal_prev,
                                current.saturating_sub(key_size),
                                cx,
                            );
                        })),
                )
                .child(format!(
                    "{}–{}{}",
                    if keys.is_empty() { 0 } else { key_offset + 1 },
                    (key_offset + key_size).min(key_offset + keys.len()),
                    if key_response["truncated"] == true {
                        "+"
                    } else {
                        ""
                    }
                ))
                .child(
                    widgets::button("home-keys-next")
                        .label("Next")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if key_can_next {
                                this.set_discovery_offset(
                                    "keys",
                                    &key_signal_next,
                                    key_offset + key_size,
                                    cx,
                                );
                            }
                        })),
                )
                .child(
                    widgets::button("home-keys-page-size")
                        .label(format!("{key_size} per page"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let current = page_size(&this.data, &format!("keys.{key_signal_size}"));
                            let next = PAGE_SIZES[(PAGE_SIZES
                                .iter()
                                .position(|v| *v == current)
                                .unwrap_or(0)
                                + 1)
                                % PAGE_SIZES.len()];
                            this.set_discovery_page_size("keys", &key_signal_size, next, cx);
                        })),
                ),
        );
        content = content.child(key_list);

        let values = discovery_rows(self.data.get("home.values").unwrap_or(&Value::Null));
        let value_response = self.data.get("home.values").cloned().unwrap_or(Value::Null);
        let selected_key_label = if selected_key.is_null() {
            "Choose an attribute".into()
        } else {
            field(&selected_key, "key")
        };
        let mut value_list = div()
            .id("attribute-values")
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .bg(color(p.panel))
            .rounded_md()
            .child(widgets::heading(
                format!(
                    "{selected_key_label} · {} values{}",
                    values.len(),
                    if value_response["truncated"] == true {
                        "+"
                    } else {
                        ""
                    }
                ),
                p,
            ));
        let value_state = format!("values.{signal}");
        let value_size = page_size(&self.data, &value_state);
        let value_offset = page_offset(&self.data, &value_state);
        for (index, value) in values
            .iter()
            .skip(value_offset)
            .take(value_size)
            .enumerate()
        {
            let typed = value["value"].clone();
            let label = attribute(&typed);
            let exact = label.clone();
            let ratio = number(&value["relativeFrequency"])
                .unwrap_or(0.)
                .clamp(0., 1.);
            let scope_value = typed.clone();
            value_list = value_list.child(
                div()
                    .id(("attribute-value", index))
                    .test_support()
                    .role(Role::ListItem)
                    .aria_label(format!(
                        "{}: {} of {} records",
                        field(&typed, "kind"),
                        field(value, "count"),
                        field(value, "denominator")
                    ))
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(color(p.chrome))
                    .child(
                        div()
                            .w(relative(0.28))
                            .min_w(px(70.))
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(8.))
                            .rounded_full()
                            .bg(color(p.chrome))
                            .child(
                                div()
                                    .w(relative(ratio as f32))
                                    .h_full()
                                    .rounded_full()
                                    .bg(color(p.accent)),
                            ),
                    )
                    .child(format!(
                        "{} / {} · {:.1}%",
                        field(value, "count"),
                        field(value, "denominator"),
                        ratio * 100.
                    ))
                    .child(
                        widgets::button(("scope-attribute-value", index))
                            .label("Scope")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.scope_telemetry_to_value(scope_value.clone(), cx)
                            })),
                    )
                    .child(
                        widgets::button(("copy-attribute-value", index))
                            .label("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(exact.clone()))
                            }),
                    ),
            );
        }
        if !values.is_empty() {
            let count = values.len();
            let size = value_size;
            let value_signal_prev = signal.clone();
            let value_signal_next = signal.clone();
            let value_signal_size = signal.clone();
            let value_can_next = value_response["truncated"] == true;
            value_list = value_list.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .p_2()
                    .child(
                        widgets::button("home-values-prev")
                            .label("Previous")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let current =
                                    page_offset(&this.data, &format!("values.{value_signal_prev}"));
                                this.set_discovery_offset(
                                    "values",
                                    &value_signal_prev,
                                    current.saturating_sub(size),
                                    cx,
                                );
                            })),
                    )
                    .child(format!(
                        "{}–{} / {}{}",
                        (value_offset + 1).min(count),
                        (value_offset + size).min(count),
                        count,
                        if value_response["truncated"] == true {
                            "+"
                        } else {
                            ""
                        }
                    ))
                    .child(
                        widgets::button("home-values-next")
                            .label("Next")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let current =
                                    page_offset(&this.data, &format!("values.{value_signal_next}"));
                                if value_can_next {
                                    this.set_discovery_offset(
                                        "values",
                                        &value_signal_next,
                                        current + size,
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        widgets::button("home-values-page-size")
                            .label(format!("{} per page", size))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let current =
                                    page_size(&this.data, &format!("values.{value_signal_size}"));
                                let next = PAGE_SIZES[(PAGE_SIZES
                                    .iter()
                                    .position(|v| *v == current)
                                    .unwrap_or(0)
                                    + 1)
                                    % PAGE_SIZES.len()];
                                this.set_discovery_page_size(
                                    "values",
                                    &value_signal_size,
                                    next,
                                    cx,
                                );
                            })),
                    ),
            );
        }
        if selected_key.is_null() {
            value_list = value_list.child(widgets::empty(
                "Select a key to inspect its typed values and record frequency.",
                p,
            ));
        } else if values.is_empty() {
            value_list = value_list.child(widgets::empty(
                "No values match this key and current filter.",
                p,
            ));
        }
        content = content.child(value_list);
        main.child(content).into_any_element()
    }

    fn render_service_telemetry(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(self.prefs.dark);
        let Some((_, _)) = self.selected_service.as_ref() else {
            return div().into_any_element();
        };
        let active = state_text(&self.data, "home.signal", "traces");
        let service = home_service_rows(&self.data)
            .into_iter()
            .find(|row| service_identity(row) == selected_identity(&self.data).unwrap_or_default())
            .unwrap_or(Value::Null);
        let mut details = div()
            .id("home-details")
            .test_support()
            .role(Role::Group)
            .aria_label("Service telemetry")
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col();
        let scoped = self.data.get("home.telemetry-scope").cloned();
        if let Some(scope) = scoped.as_ref() {
            details = details.child(div().px_2().child(widgets::property(
                "Scoped to",
                format!("{} = {}", field(scope, "key"), attribute(&scope["value"])),
                p,
            )));
        }
        let mut tabs = div()
            .id("home-telemetry-tabs")
            .h(px(48.))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_lg()
            .bg(color(p.panel));
        for signal in SIGNALS {
            let is_active = active == signal;
            let count = service_count(&service, signal);
            let label = match signal {
                "traces" => "Traces",
                "metrics" => "Metrics",
                _ => "Logs",
            };
            tabs = tabs.child(
                widgets::button(format!("home-telemetry-tab-{signal}"))
                    .label(format!("{label} [{count}]"))
                    .selected(is_active)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.data.insert("home.signal".into(), json!(signal));
                        cx.notify();
                    })),
            );
        }
        details = details.child(tabs);
        let sort = state_text(&self.data, &format!("home.sort.{active}"), "time");
        let direction = state_text(&self.data, &format!("home.sort-direction.{active}"), "desc");
        let size = page_size(&self.data, &active);
        let sort_signal = active.clone();
        let size_signal = active.clone();
        details = details.child(
            div()
                .h(px(40.))
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .px_2()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(color(p.subtle))
                        .child(format!(
                            "{} · {} records",
                            match active.as_str() {
                                "traces" => "Traces",
                                "metrics" => "Metrics",
                                _ => "Logs",
                            },
                            field(
                                &service,
                                match active.as_str() {
                                    "traces" => "traceCount",
                                    "metrics" => "metricCount",
                                    _ => "logCount",
                                }
                            )
                        )),
                )
                .child(
                    widgets::button("home-telemetry-sort")
                        .label(format!("Sort: {sort} {direction}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cycle_telemetry_sort(&sort_signal, cx)
                        })),
                )
                .child(
                    widgets::button("home-telemetry-page-size")
                        .label(format!("{size} per page"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let current = page_size(&this.data, &size_signal);
                            let next = PAGE_SIZES[(PAGE_SIZES
                                .iter()
                                .position(|v| *v == current)
                                .unwrap_or(0)
                                + 1)
                                % PAGE_SIZES.len()];
                            this.set_home_page_size(&size_signal, next, cx);
                        })),
                ),
        );
        let records = self
            .data
            .get(&format!("home.{active}"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let records = sorted_records(records, &active, &sort, &direction);
        let offset = page_offset(&self.data, &active);
        let total = total_label(&service, &active, records.len());
        let page_records = records
            .iter()
            .skip(offset)
            .take(size)
            .cloned()
            .collect::<Vec<_>>();
        let mut list = div().id("service-telemetry").flex_1().overflow_y_scroll();
        for (index, record) in page_records.iter().enumerate() {
            let id = match active.as_str() {
                "traces" => field(record, "traceID"),
                "logs" => field(record, "logRef"),
                _ => field(record, "metricRef"),
            };
            let signal = active.clone();
            list = list.child(
                div()
                    .id(("home-record", index))
                    .p_3()
                    .m_2()
                    .rounded_md()
                    .bg(color(p.canvas))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| match signal.as_str() {
                        "traces" => this.select_trace(id.clone(), cx),
                        "logs" => this.select_log(id.clone(), cx),
                        _ => this.select_metric(id.clone(), cx),
                    }))
                    .child(div().text_ellipsis().child(record_title(record, &active)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(color(p.subtle))
                            .child(record_subtitle(record, &active)),
                    ),
            );
        }
        if records.is_empty() {
            list = list.child(widgets::empty(
                format!("No {active} match this service, time range, and query."),
                p,
            ));
        }
        let prev_signal = active.clone();
        let next_signal = active.clone();
        let row_count = records.len();
        details = details.child(list).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .p_2()
                .child(
                    widgets::button("home-telemetry-prev")
                        .label("Previous")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let current = page_offset(&this.data, &prev_signal);
                            this.set_home_offset(&prev_signal, current.saturating_sub(size), cx);
                        })),
                )
                .child(format!(
                    "{}–{} / {}{}",
                    if records.is_empty() { 0 } else { offset + 1 },
                    (offset + size).min(records.len()),
                    total,
                    if records.len() >= 500 { "+" } else { "" }
                ))
                .child(
                    widgets::button("home-telemetry-next")
                        .label("Next")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let current = page_offset(&this.data, &next_signal);
                            if current + size < row_count {
                                this.set_home_offset(&next_signal, current + size, cx);
                            }
                        })),
                ),
        );
        details.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn typed_service_identity_is_the_selection_key() {
        let string_identity = json!({"serviceIdentity":"[{\"kind\":\"string\",\"value\":\"7\"}, {\"kind\":\"string\",\"value\":\"\"}]"});
        let integer_identity = json!({"serviceIdentity":"[{\"kind\":\"int64\",\"value\":\"7\"}, {\"kind\":\"string\",\"value\":\"\"}]"});
        assert_ne!(
            service_identity(&string_identity),
            service_identity(&integer_identity)
        );
    }

    #[::core::prelude::v1::test]
    fn home_page_sizes_are_per_signal_and_restricted_to_supported_values() {
        let data = HashMap::from([
            ("home.page-size.traces".to_owned(), json!(100)),
            ("home.page-size.logs".to_owned(), json!(50)),
            ("home.page-size.metrics".to_owned(), json!(99)),
        ]);
        assert_eq!(page_size(&data, "traces"), 100);
        assert_eq!(page_size(&data, "logs"), 50);
        assert_eq!(page_size(&data, "metrics"), 25);
    }

    #[::core::prelude::v1::test]
    fn telemetry_scope_uses_exact_typed_service_identity_and_attribute_value() {
        let identity = r#"[{"kind":"string","value":"api"},{"kind":"int64","value":"7"}]"#;
        let scope = json!({
            "key":"region",
            "kind":"string",
            "ownerType":"resource",
            "value":{"kind":"string","value":"west"}
        });
        let query = telemetry_query(identity, Some(&scope), None);
        let children = query["group"]["children"].as_array().unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0]["query"]["field"]["name"], "serviceIdentity");
        assert_eq!(children[0]["query"]["value"], identity);
        assert_eq!(children[1]["query"]["field"]["name"], "region");
        assert_eq!(children[1]["query"]["field"]["attributeScope"], "resource");
        assert_eq!(children[1]["query"]["value"], "west");
    }

    #[::core::prelude::v1::test]
    fn discovery_page_sizes_are_retained_independently_per_signal() {
        let data = HashMap::from([
            ("home.page-size.keys.traces".to_owned(), json!(100)),
            ("home.page-size.values.logs".to_owned(), json!(50)),
        ]);
        assert_eq!(page_size(&data, "keys.traces"), 100);
        assert_eq!(page_size(&data, "values.logs"), 50);
        assert_eq!(page_size(&data, "keys.logs"), 25);
    }

    #[::core::prelude::v1::test]
    fn owner_choices_follow_the_signal_data_model() {
        assert!(owner_options("traces").contains(&"event"));
        assert!(owner_options("logs").contains(&"log"));
        assert!(owner_options("metrics").contains(&"exemplar"));
        assert!(!owner_options("logs").contains(&"exemplar"));
    }

    #[::core::prelude::v1::test]
    fn service_map_edge_hit_test_selects_nearest_segment_with_tolerance() {
        let edge = json!({"id":"edge-a"});
        let edges = vec![(edge.clone(), (0.1, 0.5), (0.9, 0.5))];
        assert_eq!(
            hit_test_connection(&edges, (0.55, 0.508), 0.02),
            Some(edge.clone())
        );
        assert_eq!(hit_test_connection(&edges, (0.55, 0.55), 0.02), None);
        assert_eq!(
            point_segment_distance((0.5, 0.5), (0.5, 0.5), (0.5, 0.5)),
            0.
        );
    }

    #[::core::prelude::v1::test]
    fn per_key_expansion_overrides_bulk_attribute_state() {
        let id = "service-signal-key";
        let data = HashMap::from([
            ("home.expanded-override.another-key".to_owned(), json!(true)),
            (format!("home.expanded-override.{id}"), json!(false)),
        ]);
        assert!(!attribute_entry_expanded(&data, id));
        assert!(attribute_entry_expanded(&data, "another-key"));
    }

    #[::core::prelude::v1::test]
    fn home_service_pages_continue_offsets_and_deduplicate_typed_identities() {
        let first = json!({"rows":[
            [{"serviceIdentity":"string-api","serviceName":"api"}],
            [{"serviceIdentity":"int-api","serviceName":"api"}]],"truncated":true});
        let second = json!({"rows":[
            [{"serviceIdentity":"int-api","serviceName":"api"}],
            [{"serviceIdentity":"worker","serviceName":"worker"}]],"truncated":false});
        let data = HashMap::from([
            ("home.services".to_owned(), first),
            ("home.services.more.2".to_owned(), second),
        ]);
        let rows = home_service_rows(&data);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["serviceIdentity"], "string-api");
        assert_eq!(rows[1]["serviceIdentity"], "int-api");
        assert_eq!(rows[2]["serviceIdentity"], "worker");
        assert_eq!(next_home_services_offset(&data), 4);
        assert!(!home_services_have_more(&data));
    }

    #[::core::prelude::v1::test]
    fn home_more_services_appears_only_when_latest_page_is_truncated() {
        let data = HashMap::from([(
            "home.services".to_owned(),
            json!({"rows":[[{"serviceIdentity":"api"}]],"truncated":true}),
        )]);
        assert!(home_services_have_more(&data));
        assert_eq!(next_home_services_offset(&data), 1);
    }
}

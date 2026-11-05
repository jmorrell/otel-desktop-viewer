//! Projection of received histograms and scalar views into native chart geometry.
//! Received integers remain strings in details; only screen coordinates are f64.
use crate::model::{attribute, field, ns, number};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Bin {
    pub lower: f64,
    pub upper: f64,
    pub count: f64,
    /// Count exactly as supplied on the wire; geometry may round the f64.
    pub exact_count: String,
}

pub fn histogram_bins(value: &Value) -> Vec<Bin> {
    if let Some(counts) = value["bucketCounts"].as_array() {
        let bounds: Vec<f64> = value["explicitBounds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(number)
            .collect();
        return counts
            .iter()
            .enumerate()
            .map(|(i, count)| Bin {
                lower: if i == 0 {
                    f64::NEG_INFINITY
                } else {
                    bounds.get(i - 1).copied().unwrap_or(f64::INFINITY)
                },
                upper: bounds.get(i).copied().unwrap_or(f64::INFINITY),
                count: number(count).unwrap_or(0.),
                exact_count: count
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| count.to_string()),
            })
            .collect();
    }
    let scale = number(&value["scale"]).unwrap_or(0.);
    let exponent = 2_f64.powf(-scale);
    let mut bins = Vec::new();
    for positive in [false, true] {
        let (counts, offset) = if positive {
            (
                &value["positiveBucketCounts"],
                number(&value["positiveBucketOffset"]).unwrap_or(0.),
            )
        } else {
            (
                &value["negativeBucketCounts"],
                number(&value["negativeBucketOffset"]).unwrap_or(0.),
            )
        };
        for (i, count) in counts.as_array().into_iter().flatten().enumerate() {
            let lower = 2_f64.powf((offset + i as f64) * exponent);
            let upper = 2_f64.powf((offset + i as f64 + 1.) * exponent);
            bins.push(if positive {
                Bin {
                    lower,
                    upper,
                    count: number(count).unwrap_or(0.),
                    exact_count: count
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| count.to_string()),
                }
            } else {
                Bin {
                    lower: -upper,
                    upper: -lower,
                    count: number(count).unwrap_or(0.),
                    exact_count: count
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| count.to_string()),
                }
            });
        }
    }
    if let Some(count) = number(&value["zeroCount"]).filter(|v| *v > 0.) {
        let t = number(&value["zeroThreshold"]).unwrap_or(0.);
        bins.push(Bin {
            lower: -t,
            upper: t,
            count,
            exact_count: value["zeroCount"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value["zeroCount"].to_string()),
        });
    }
    bins.sort_by(|a, b| a.lower.total_cmp(&b.lower));
    bins
}

#[derive(Clone)]
pub struct PlotPoint {
    pub time: i128,
    pub value: f64,
    pub display: String,
    pub source: String,
    pub reset: bool,
    pub slope: Option<f64>,
}
#[derive(Clone)]
pub struct Curve {
    pub id: String,
    pub label: String,
    pub points: Vec<PlotPoint>,
    pub color: u32,
}
#[derive(Clone)]
pub struct HeatColumn {
    pub time: i128,
    pub start: i128,
    pub bins: Vec<Bin>,
    pub value: Value,
}
#[derive(Clone, Default)]
pub struct ChartData {
    pub curves: Vec<Curve>,
    pub columns: Vec<HeatColumn>,
    pub x_min: i128,
    pub x_max: i128,
    pub y_min: f64,
    pub y_max: f64,
    pub count_max: f64,
}

impl ChartData {
    pub fn from_view(
        view: &Value,
        mode: usize,
        visible: &std::collections::HashSet<String>,
        aggregate: bool,
        quantiles: bool,
        color: impl Fn(&str) -> u32,
    ) -> Self {
        let histogram = field(view, "metricType").contains("Histogram");
        let mut data = Self::default();
        for series in view["timeseries"].as_array().into_iter().flatten() {
            let id = field(series, "seriesRef");
            if !visible.contains(&id) {
                continue;
            }
            let points = if mode != 3 {
                series["datapoints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|point| {
                        let value =
                            number(&point["doubleValue"]).or_else(|| number(&point["intValue"]))?;
                        value.is_finite().then(|| PlotPoint {
                            time: ns(&point["timestamp"]),
                            value,
                            display: point["intValue"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| value.to_string()),
                            source: field(point, "id"),
                            // The server compares cumulative integer values before
                            // floating-point projection and reports the reset here.
                            reset: point["isReset"].as_bool().unwrap_or(false),
                            slope: None,
                        })
                    })
                    .collect()
            } else {
                series["views"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|point| {
                        let value = number(
                            &point[match mode {
                                1 => "sum",
                                2 => "avg",
                                _ => "rate",
                            }],
                        )?;
                        value.is_finite().then(|| PlotPoint {
                            time: ns(&point["bucketStart"]),
                            value,
                            display: value.to_string(),
                            source: String::new(),
                            reset: point["hasReset"].as_bool().unwrap_or(false),
                            slope: number(&point["slope"]),
                        })
                    })
                    .collect()
            };
            if !histogram {
                data.curves.push(Curve {
                    label: series["attributes"]
                        .as_array()
                        .map(|attrs| {
                            attrs
                                .iter()
                                .take(3)
                                .map(|a| format!("{}={}", field(a, "key"), attribute(&a["value"])))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| id.chars().take(8).collect()),
                    id: id.clone(),
                    points,
                    color: color(&id),
                });
            }
        }
        if !histogram && mode > 0 {
            let fields: &[&str] = match mode {
                1 => &["sum"],
                2 => &["avg"],
                3 => &["rate"],
                _ => &["sum", "avg"],
            };
            for key in if aggregate {
                &["all", "selected"][..]
            } else {
                &["selected"][..]
            } {
                for metric_field in fields {
                    let points = view["scalarAggregate"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|point| {
                            let value = number(&point[metric_field])?;
                            value.is_finite().then(|| PlotPoint {
                                time: ns(&point["bucketStart"]),
                                value,
                                display: value.to_string(),
                                source: String::new(),
                                reset: point["hasReset"].as_bool().unwrap_or(false),
                                slope: number(&point["slope"]),
                            })
                        })
                        .collect();
                    let id = format!("aggregate:{key}:{metric_field}");
                    data.curves.push(Curve {
                        id: id.clone(),
                        label: format!("{metric_field} · {key}"),
                        points,
                        color: color(&id),
                    });
                }
            }
        }
        if histogram {
            data.columns = view["aggregate"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|bucket| HeatColumn {
                    time: ns(&bucket["timestamp"]),
                    start: ns(&bucket["startTime"]),
                    bins: histogram_bins(bucket),
                    value: bucket.clone(),
                })
                .collect();
            if quantiles {
                for q in ["0.5", "0.9", "0.95", "0.99"] {
                    let points = data
                        .columns
                        .iter()
                        .filter_map(|c| {
                            number(&c.value["quantiles"][q])
                                .filter(|v| v.is_finite())
                                .map(|value| PlotPoint {
                                    time: c.time,
                                    value,
                                    display: value.to_string(),
                                    source: String::new(),
                                    reset: false,
                                    slope: None,
                                })
                        })
                        .collect();
                    data.curves.push(Curve {
                        id: format!("p{q}"),
                        label: format!("p{}", q.parse::<f64>().unwrap() * 100.),
                        points,
                        color: color(q),
                    });
                }
            }
        }
        let times: Vec<i128> = data
            .curves
            .iter()
            .flat_map(|c| c.points.iter().map(|p| p.time))
            .chain(data.columns.iter().flat_map(|c| [c.start, c.time]))
            .collect();
        data.x_min = view["window"]["effective"]["startNs"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| times.iter().copied().min().unwrap_or(0));
        data.x_max = view["window"]["effective"]["endNs"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| times.iter().copied().max().unwrap_or(1));
        if data.x_min >= data.x_max {
            data.x_min = times.iter().copied().min().unwrap_or(data.x_min);
            data.x_max = times.iter().copied().max().unwrap_or(data.x_min + 1);
            if data.x_min >= data.x_max {
                data.x_min -= 500_000_000;
                data.x_max += 500_000_000;
            }
        }
        let extents = data
            .columns
            .iter()
            .flat_map(|c| {
                c.bins
                    .iter()
                    .filter(|b| b.count > 0.)
                    .flat_map(|b| [b.lower, b.upper])
            })
            .filter(|v| v.is_finite());
        let values: Vec<f64> = data
            .curves
            .iter()
            .flat_map(|c| c.points.iter().map(|p| p.value))
            .chain(extents)
            .collect();
        data.y_min = values.iter().copied().reduce(f64::min).unwrap_or(0.);
        data.y_max = values.iter().copied().reduce(f64::max).unwrap_or(1.);
        if !histogram {
            data.y_min = data.y_min.min(0.);
            data.y_max = data.y_max.max(0.);
        }
        if data.y_min == data.y_max {
            let padding = data.y_min.abs().max(1.) * 0.1;
            data.y_min -= padding;
            data.y_max += padding;
        }
        if histogram {
            let pad = (data.y_max - data.y_min) * 0.05;
            data.y_min -= pad;
            data.y_max += pad;
        }
        data.count_max = data
            .columns
            .iter()
            .flat_map(|c| c.bins.iter().map(|b| b.count))
            .fold(1., f64::max);
        data
    }
    pub fn x(&self, time: i128) -> f32 {
        ((time - self.x_min) as f64 / (self.x_max - self.x_min).max(1) as f64).clamp(0., 1.) as f32
    }
    pub fn y(&self, value: f64) -> f32 {
        (1. - (value - self.y_min) / (self.y_max - self.y_min)).clamp(0., 1.) as f32
    }
    pub fn nearest_xy(&self, x: f32, y: f32) -> Option<(&Curve, &PlotPoint)> {
        self.curves
            .iter()
            .flat_map(|c| c.points.iter().map(move |p| (c, p)))
            .min_by(|(_, a), (_, b)| {
                let distance = |p: &PlotPoint| {
                    ((self.x(p.time) - x) * 2.).powi(2) + (self.y(p.value) - y).powi(2)
                };
                distance(a).total_cmp(&distance(b))
            })
    }
    pub fn nearest_column(&self, x: f32) -> Option<&HeatColumn> {
        let target = self.x_min + ((self.x_max - self.x_min) as f64 * x as f64) as i128;
        self.columns
            .iter()
            .find(|c| c.start <= target && target <= c.time)
            .or_else(|| {
                self.columns
                    .iter()
                    .min_by_key(|c| (c.time - target).unsigned_abs())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn explicit_histogram_keeps_overflow_and_exact_count_conversion() {
        let bins = histogram_bins(&json!({"explicitBounds":[0,10],"bucketCounts":["2","3","4"]}));
        assert_eq!(bins.len(), 3);
        assert_eq!(bins[0].lower, f64::NEG_INFINITY);
        assert_eq!(bins[2].upper, f64::INFINITY);
        assert_eq!(bins.iter().map(|b| b.count).sum::<f64>(), 9.);
    }
    #[test]
    fn exponential_negative_buckets_have_correct_orientation() {
        let bins = histogram_bins(
            &json!({"scale":0,"positiveBucketOffset":0,"positiveBucketCounts":["3","4"],"negativeBucketOffset":0,"negativeBucketCounts":["2"],"zeroCount":"1","zeroThreshold":0.1}),
        );
        assert_eq!((bins[0].lower, bins[0].upper), (-2., -1.));
        assert_eq!((bins[2].lower, bins[2].upper), (1., 2.));
        assert_eq!(bins.iter().map(|b| b.count).sum::<f64>(), 10.);
    }
    #[test]
    fn no_checked_series_means_no_received_curves() {
        let view = json!({"metricType":"Gauge","timeseries":[{"seriesRef":"a","datapoints":[{"timestamp":"10","intValue":"3","id":"point"}]}]});
        let data = ChartData::from_view(&view, 0, &Default::default(), false, false, |_| 0);
        assert!(data.curves.is_empty());
        let data = ChartData::from_view(
            &view,
            0,
            &std::collections::HashSet::from(["a".into()]),
            false,
            false,
            |_| 0,
        );
        assert_eq!(data.curves[0].points[0].source, "point");
    }
    #[test]
    fn chart_subtracts_epoch_before_float_conversion() {
        let mut data = ChartData::default();
        data.x_min = 1_788_000_000_000_000_000;
        data.x_max = data.x_min + 2;
        assert_eq!(data.x(data.x_min + 1), 0.5);
    }
    #[test]
    fn scalar_hover_keeps_received_int64_and_selects_the_correct_series() {
        let view = json!({"metricType":"Gauge","timeseries":[
            {"seriesRef":"large","datapoints":[{"timestamp":"1788000000000000000","intValue":"9223372036854775807","id":"exact-point"}]},
            {"seriesRef":"small","datapoints":[{"timestamp":"1788000000000000000","intValue":"1","id":"other-point"}]}
        ]});
        let data = ChartData::from_view(
            &view,
            0,
            &std::collections::HashSet::from(["large".into(), "small".into()]),
            false,
            false,
            |_| 0,
        );
        assert_eq!(data.curves[0].points[0].display, "9223372036854775807");
        let (series, point) = data.nearest_xy(0.5, 0.).unwrap();
        assert_eq!(series.id, "large");
        assert_eq!(point.source, "exact-point");
        let (series, _) = data.nearest_xy(0.5, 1.).unwrap();
        assert_eq!(series.id, "small");
    }
    #[test]
    fn reset_flags_use_server_comparison_before_float_rounding() {
        let view = json!({"metricType":"Sum","timeseries":[{"seriesRef":"counter","datapoints":[
            {"timestamp":"1","intValue":"9007199254740993","id":"before","isReset":false},
            {"timestamp":"2","intValue":"9007199254740992","id":"after","isReset":true}
        ]}]});
        let data = ChartData::from_view(
            &view,
            0,
            &std::collections::HashSet::from(["counter".into()]),
            false,
            false,
            |_| 0,
        );
        assert!(!data.curves[0].points[0].reset);
        assert!(data.curves[0].points[1].reset);
        assert_eq!(data.curves[0].points[1].display, "9007199254740992");
    }
    #[test]
    fn histogram_hover_count_keeps_exact_wire_integers() {
        let bins = histogram_bins(
            &json!({"scale":0,"positiveBucketOffset":0,"positiveBucketCounts":["9007199254740993"],"negativeBucketOffset":0,"negativeBucketCounts":["9007199254740995"],"zeroCount":"9007199254740997","zeroThreshold":0.1}),
        );
        assert_eq!(
            bins.iter()
                .map(|b| b.exact_count.as_str())
                .collect::<Vec<_>>(),
            vec!["9007199254740995", "9007199254740997", "9007199254740993"]
        );
    }
    #[test]
    fn singleton_heatmap_has_a_visible_domain_and_keeps_its_interval() {
        let mut view = json!({"metricType":"Histogram","aggregate":[{
            "timestamp":"1788000000000000000","startTime":"1787999999000000000",
            "explicitBounds":[1],"bucketCounts":[2,0]
        }],"window":{"effective":{"startNs":"1788000000000000000","endNs":"1788000000000000000"}}});
        let data = ChartData::from_view(&view, 0, &Default::default(), false, false, |_| 0);
        assert_eq!(data.x_max - data.x_min, 1_000_000_000);
        assert_eq!(data.x(data.columns[0].start), 0.);
        assert_eq!(data.x(data.columns[0].time), 1.);
        view["aggregate"][0]["startTime"] = view["aggregate"][0]["timestamp"].clone();
        let data = ChartData::from_view(&view, 0, &Default::default(), false, false, |_| 0);
        assert!(data.x_max > data.x_min);
        assert_eq!(data.x(data.columns[0].time), 0.5);
    }
}

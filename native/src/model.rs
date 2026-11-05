//! Exact wire-value helpers and the viewer's search grammar.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}
pub fn field(value: &Value, key: &str) -> String {
    text(&value[key])
}
pub fn ns(value: &Value) -> i128 {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_i64().map(i128::from))
        .or_else(|| value.as_u64().map(i128::from))
        .unwrap_or(0)
}
pub fn number(value: &Value) -> Option<f64> {
    if value.is_null() {
        return None;
    }
    if let Some(v) = value.as_f64() {
        return Some(v);
    }
    let s = value.as_str()?;
    if let Some(bits) = s.strip_prefix("0x") {
        u64::from_str_radix(bits, 16).ok().map(f64::from_bits)
    } else {
        s.parse().ok()
    }
}
pub fn format_duration(value: i128) -> String {
    let a = value.unsigned_abs();
    let (scale, unit) = if a >= 60_000_000_000 {
        (60_000_000_000.0, "min")
    } else if a >= 1_000_000_000 {
        (1_000_000_000.0, "s")
    } else if a >= 1_000_000 {
        (1_000_000.0, "ms")
    } else if a >= 1_000 {
        (1_000.0, "µs")
    } else {
        return format!("{value} ns");
    };
    format!("{:.3} {unit}", value as f64 / scale)
}
static DISPLAY_TIMEZONE: std::sync::RwLock<Option<chrono_tz::Tz>> = std::sync::RwLock::new(None);
pub fn set_timezone(name: &str) -> Result<()> {
    let zone = if name.eq_ignore_ascii_case("local") || name.is_empty() {
        None
    } else {
        Some(
            name.parse::<chrono_tz::Tz>()
                .map_err(|_| anyhow::anyhow!("Unknown IANA timezone: {name}"))?,
        )
    };
    *DISPLAY_TIMEZONE.write().unwrap() = zone;
    Ok(())
}
/// Include the IANA name so the store aligns buckets correctly across DST.
pub fn timezone_rpc_params() -> (i64, Option<String>) {
    use chrono::Offset;
    let zone = *DISPLAY_TIMEZONE.read().unwrap();
    if let Some(zone) = zone {
        let seconds = chrono::Utc::now()
            .with_timezone(&zone)
            .offset()
            .fix()
            .local_minus_utc();
        return ((seconds as i64) * 1_000_000_000, Some(zone.name().into()));
    }
    let local = std::env::var("TZ")
        .ok()
        .filter(|s| s.parse::<chrono_tz::Tz>().is_ok())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|p| {
                p.to_str()
                    .and_then(|s| s.split_once("zoneinfo/").map(|(_, name)| name.to_owned()))
            })
        });
    (
        (chrono::Local::now().offset().local_minus_utc() as i64) * 1_000_000_000,
        local,
    )
}
pub fn format_timestamp(value: i128) -> String {
    use chrono::{DateTime, Local};
    let seconds = value.div_euclid(1_000_000_000);
    let subsec = value.rem_euclid(1_000_000_000) as u32;
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, subsec))
        .map(|dt| {
            if let Some(zone) = *DISPLAY_TIMEZONE.read().unwrap() {
                dt.with_timezone(&zone)
                    .format("%Y-%m-%d %H:%M:%S%.3f %Z")
                    .to_string()
            } else {
                dt.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S%.3f %Z")
                    .to_string()
            }
        })
        .unwrap_or_else(|| format!("{value} ns"))
}
pub fn attribute(value: &Value) -> String {
    let v = &value["value"];
    match value["kind"].as_str() {
        Some("empty") => "null".into(),
        Some("array") => format!(
            "[{}]",
            v.as_array()
                .map(|a| a.iter().map(attribute).collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        ),
        Some("map") => format!(
            "{{{}}}",
            v.as_array()
                .map(|a| a
                    .iter()
                    .map(|e| format!("{}: {}", field(e, "key"), attribute(&e["value"])))
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        ),
        Some("double") if v.as_str().is_some_and(|s| s.starts_with("0x")) => number(v)
            .map(|f| {
                if f == 0.0 && f.is_sign_negative() {
                    "-0".into()
                } else {
                    f.to_string()
                }
            })
            .unwrap_or_else(|| text(v)),
        Some(_) => text(v),
        None => text(value),
    }
}
pub fn service(resource: &Value) -> String {
    resource["attributes"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["key"] == "service.name"))
        .map(|a| attribute(&a["value"]))
        .unwrap_or_else(|| "(unknown service)".into())
}
pub fn discovery_rows(value: &Value) -> Vec<Value> {
    value["rows"]
        .as_array()
        .map(|rows| rows.iter().map(|row| row[0].clone()).collect())
        .unwrap_or_default()
}
pub fn condition(field: Value, operator: &str, value: &str) -> Value {
    json!({"id":"native", "type":"condition", "query":{"field":field,"fieldOperator":operator,"value":value}})
}
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Symbol(String),
}
impl Token {
    fn value(&self) -> &str {
        match self {
            Self::Word(s) | Self::Quoted(s) | Self::Symbol(s) => s,
        }
    }
}
fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut chars = input.chars().peekable();
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '"' || c == '\'' {
            let mut s = String::new();
            let mut closed = false;
            while let Some(n) = chars.next() {
                if n == c {
                    closed = true;
                    break;
                }
                if n == '\\' {
                    let e = chars.next().context("Unterminated escape")?;
                    match e {
                        'n' => s.push('\n'),
                        'r' => s.push('\r'),
                        't' => s.push('\t'),
                        'b' => s.push('\u{8}'),
                        'f' => s.push('\u{c}'),
                        'u' => {
                            let digits: String = chars.clone().take(4).collect();
                            if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_hexdigit()) {
                                for _ in 0..4 {
                                    chars.next();
                                }
                                let high = u32::from_str_radix(&digits, 16)?;
                                let code = if (0xd800..=0xdbff).contains(&high) {
                                    let suffix: String = chars.clone().take(6).collect();
                                    if suffix.starts_with("\\u") && suffix.len() == 6 {
                                        let low = u32::from_str_radix(&suffix[2..], 16)
                                            .context("Invalid Unicode surrogate")?;
                                        if !(0xdc00..=0xdfff).contains(&low) {
                                            bail!("Invalid Unicode surrogate pair");
                                        }
                                        for _ in 0..6 {
                                            chars.next();
                                        }
                                        0x10000 + ((high - 0xd800) << 10) + low - 0xdc00
                                    } else {
                                        bail!("Incomplete Unicode surrogate pair");
                                    }
                                } else {
                                    high
                                };
                                s.push(char::from_u32(code).context("Invalid Unicode escape")?);
                            } else {
                                s.push('u');
                            }
                        }
                        _ => s.push(e),
                    }
                } else {
                    s.push(n);
                }
            }
            if !closed {
                bail!("Unterminated quoted value");
            }
            out.push(Token::Quoted(s));
        } else if "()[],=<>!^$|".contains(c) {
            let mut s = c.to_string();
            if chars.peek().is_some_and(|n| *n == '=' || *n == '~') && "!=<>".contains(c) {
                s.push(chars.next().unwrap());
            }
            out.push(Token::Symbol(s));
        } else {
            let mut s = c.to_string();
            while chars
                .peek()
                .is_some_and(|n| !n.is_whitespace() && !"()[],=<>!^$|\"'".contains(*n))
            {
                s.push(chars.next().unwrap());
            }
            out.push(Token::Word(s));
        }
    }
    Ok(out)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    signal: &'a str,
    fields: &'a [Value],
}
impl Parser<'_> {
    fn at(&self, s: &str) -> bool {
        self.tokens
            .get(self.position)
            .is_some_and(|t| !matches!(t, Token::Quoted(_)) && t.value().eq_ignore_ascii_case(s))
    }
    fn consume(&mut self, s: &str) -> bool {
        if self.at(s) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<()> {
        if self.consume(s) {
            Ok(())
        } else {
            bail!("Expected {s}")
        }
    }
    fn next(&mut self) -> Result<Token> {
        let t = self
            .tokens
            .get(self.position)
            .cloned()
            .context("Expected a search value")?;
        self.position += 1;
        Ok(t)
    }
    fn expression(&mut self) -> Result<Value> {
        let mut values = vec![self.and()?];
        while self.consume("OR") {
            values.push(self.and()?);
        }
        Ok(group("OR", values))
    }
    fn and(&mut self) -> Result<Value> {
        let mut values = vec![self.atom()?];
        while self.consume("AND") {
            values.push(self.atom()?);
        }
        Ok(group("AND", values))
    }
    fn atom(&mut self) -> Result<Value> {
        if self.consume("(") {
            let value = self.expression()?;
            self.expect(")")?;
            return Ok(value);
        }
        let first = self.next()?;
        let mut field = json!({"searchScope":"global"});
        let mut explicit = false;
        if first.value().eq_ignore_ascii_case("attr") && self.consume("(") {
            let owner = self.next()?.value().to_owned();
            self.expect(",")?;
            let key = self.next()?.value().to_owned();
            self.expect(",")?;
            let kind = self.next()?.value().to_owned();
            self.expect(")")?;
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
            .contains(&owner.as_str())
            {
                bail!("Unknown attribute owner: {owner}");
            }
            if ![
                "string", "int64", "double", "bool", "bytes", "slice", "map", "empty",
            ]
            .contains(&kind.as_str())
            {
                bail!("Unknown attribute type: {kind}");
            }
            field =
                json!({"name":key,"type":kind,"searchScope":"attribute","attributeScope":owner});
            explicit = true;
        } else if first.value().eq_ignore_ascii_case("text") && self.consume("(") {
            let value = self.next()?.value().to_owned();
            self.expect(")")?;
            return Ok(condition(field, "CONTAINS", &value));
        }
        let mut operator = None;
        for candidate in [
            "!=", ">=", "<=", "=~", "!~", "=", ">", "<", "^", "$", "CONTAINS", "REGEXP", "IN",
        ] {
            if self.consume(candidate) {
                operator = Some(candidate.to_owned());
                break;
            }
        }
        if self.consume("NOT") {
            if self.consume("IN") {
                operator = Some("NOT IN".into());
            } else if self.consume("CONTAINS") {
                operator = Some("NOT CONTAINS".into());
            } else if self.consume("REGEXP") {
                operator = Some("NOT REGEXP".into());
            } else {
                bail!("Expected IN, CONTAINS or REGEXP after NOT");
            }
        }
        let Some(mut operator) = operator else {
            if explicit {
                bail!("Expected an operator after attribute reference");
            }
            return Ok(condition(field, "CONTAINS", first.value()));
        };
        if !explicit {
            let fields: Value = serde_json::from_str(include_str!("fields.json"))?;
            let signal = match self.signal {
                "traces" => "SPAN",
                "logs" => "LOG",
                _ => "METRIC",
            };
            let builtin = ["RESOURCE", "SCOPE", signal]
                .into_iter()
                .filter_map(|key| fields[key].as_array())
                .flatten()
                .find(|f| {
                    f["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(first.value()))
                })
                .cloned();
            let matches: Vec<_> = self
                .fields
                .iter()
                .filter(|f| {
                    f["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(first.value()))
                })
                .collect();
            field = if let Some(builtin) = builtin {
                builtin
            } else if let Some(first_match) = matches.first() {
                if matches.iter().any(|f| {
                    f["type"] != first_match["type"]
                        || f["attributeScope"] != first_match["attributeScope"]
                }) {
                    bail!(
                        "Ambiguous attribute {}. Use attr(owner, \"key\", type) to choose its owner and type.",
                        first.value()
                    );
                }
                let mut definition = (*first_match).clone();
                definition["searchScope"] = json!("attribute");
                definition
            } else if ["service.name", "service.namespace"].contains(&first.value()) {
                json!({"name":first.value(),"type":"string","searchScope":"attribute","attributeScope":"resource"})
            } else {
                bail!(
                    "Unknown field {}. Use attr(owner, \"key\", type) for attributes.",
                    first.value()
                );
            };
        }
        let is_list = self.at("[");
        if ["IN", "NOT IN"].contains(&operator.as_str()) && !is_list {
            bail!("IN and NOT IN require a list");
        }
        let mut value = if self.consume("[") {
            let mut values = Vec::new();
            while !self.consume("]") {
                let v = self.next()?;
                if !matches!(v, Token::Quoted(_))
                    && ["NULL", "NIL", "[", "]"]
                        .iter()
                        .any(|s| v.value().eq_ignore_ascii_case(s))
                {
                    bail!("Search lists must be flat and cannot contain NULL");
                }
                values.push(v.value().to_owned());
                if !self.at("]") {
                    self.expect(",")?;
                }
            }
            if values.is_empty() {
                bail!("Search lists cannot be empty");
            }
            serde_json::to_string(&values)?
        } else {
            let v = self.next()?;
            if !matches!(v, Token::Quoted(_))
                && ["NULL", "NIL"]
                    .iter()
                    .any(|s| v.value().eq_ignore_ascii_case(s))
            {
                operator = match operator.as_str() {
                    "=" => "IS NULL".into(),
                    "!=" => "IS NOT NULL".into(),
                    _ => {
                        bail!("NULL requires = or !=")
                    }
                };
                String::new()
            } else {
                v.value().to_owned()
            }
        };
        if !["IS NULL", "IS NOT NULL"].contains(&operator.as_str()) {
            let normalize = |input: &str| -> Result<String> {
                let name = field["name"].as_str().unwrap_or("");
                if name == "duration" && field["searchScope"] == "field" {
                    return normalize_duration(input);
                }
                if field["type"] == "int64" {
                    let integer = exact_integer(input)?;
                    let unsigned = [
                        "timestamp",
                        "observedTimestamp",
                        "startTime",
                        "endTime",
                        "event.timestamp",
                    ]
                    .contains(&name)
                        && field["searchScope"] == "field";
                    if unsigned {
                        if !(0..=u64::MAX as i128).contains(&integer) {
                            bail!("Timestamp must fit an unsigned 64-bit integer");
                        }
                    } else if !(i64::MIN as i128..=i64::MAX as i128).contains(&integer) {
                        bail!("Value must fit a signed 64-bit integer");
                    }
                    return Ok(integer.to_string());
                }
                Ok(input.to_owned())
            };
            if is_list {
                let list: Vec<String> = serde_json::from_str(&value)?;
                value = serde_json::to_string(
                    &list
                        .iter()
                        .map(|v| normalize(v))
                        .collect::<Result<Vec<_>>>()?,
                )?;
            } else {
                value = normalize(&value)?;
            }
        }
        if operator == "=~" {
            operator = "REGEXP".into();
        }
        if operator == "!~" {
            operator = "NOT REGEXP".into();
        }
        Ok(condition(field, &operator, &value))
    }
}
/// Decimal and exponent integer syntax is normalized without floating-point rounding.
fn exact_integer(input: &str) -> Result<i128> {
    if input.len() > 256 {
        bail!("Integer literal is too long");
    }
    let (base, exponent) = input.split_once(['e', 'E']).map_or((input, 0), |(a, b)| {
        (a, b.parse::<i32>().unwrap_or(i32::MAX))
    });
    let negative = base.starts_with('-');
    let base = base.trim_start_matches(['+', '-']);
    let (whole, fraction) = base.split_once('.').unwrap_or((base, ""));
    let digits = format!("{whole}{fraction}");
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        bail!("Expected an exact integer");
    }
    let mut value = digits
        .parse::<i128>()
        .context("Integer literal is out of range")?;
    if value == 0 {
        return Ok(0);
    }
    let scale = (exponent as i64) - (fraction.len() as i64);
    if scale.unsigned_abs() > 38 {
        bail!("Expected an exact 64-bit integer");
    }
    let multiplier = 10_i128
        .checked_pow(scale.unsigned_abs() as u32)
        .context("Integer literal is out of range")?;
    if scale < 0 {
        if value % multiplier != 0 {
            bail!("Expected an integer, received a fraction");
        }
        value /= multiplier;
    } else {
        value = value
            .checked_mul(multiplier)
            .context("Integer literal is out of range")?;
    }
    Ok(if negative { -value } else { value })
}
fn normalize_duration(input: &str) -> Result<String> {
    let value = input.trim();
    if value.chars().all(|c| c.is_ascii_digit()) && !value.is_empty() {
        let ns = value
            .parse::<i64>()
            .context("Duration must fit signed 64-bit nanoseconds")?;
        return Ok(ns.to_string());
    }
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .context("Duration needs a unit such as ms or s")?;
    let (numeric, unit) = value.split_at(split);
    let multiplier: i128 = match unit.trim().to_lowercase().as_str() {
        "ns" => 1,
        "us" | "µs" => 1000,
        "ms" => 1_000_000,
        "s" => 1_000_000_000,
        "m" | "min" => 60_000_000_000,
        "h" => 3_600_000_000_000,
        _ => bail!("Unknown duration unit"),
    };
    let (whole, fraction) = numeric.split_once('.').unwrap_or((numeric, ""));
    if whole.is_empty()
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
        || fraction.len() > 18
    {
        bail!("Invalid duration");
    }
    let denominator = 10_i128.pow(fraction.len() as u32);
    let numerator = format!("{whole}{fraction}")
        .parse::<i128>()?
        .checked_mul(multiplier)
        .context("Duration is too large")?;
    let ns = numerator / denominator
        + if (numerator % denominator) * 2 >= denominator {
            1
        } else {
            0
        };
    if ns > i64::MAX as i128 {
        bail!("Duration must fit signed 64-bit nanoseconds");
    }
    Ok(ns.to_string())
}
fn group(operator: &str, mut values: Vec<Value>) -> Value {
    if values.len() == 1 {
        values.pop().unwrap()
    } else {
        json!({"id":"native-group","type":"group","group":{"logicalOperator":operator,"children":values}})
    }
}
pub fn parse_search(input: &str, signal: &str) -> Result<Value> {
    Ok(parse_search_request(input, signal)?.0)
}
pub fn parse_search_request(input: &str, signal: &str) -> Result<(Value, Option<usize>)> {
    parse_search_request_with_fields(input, signal, &[])
}
pub fn parse_search_request_with_fields(
    input: &str,
    signal: &str,
    available: &[Value],
) -> Result<(Value, Option<usize>)> {
    let tokens = tokenize(input)?;
    let structured = tokens.iter().any(|token| {
        !matches!(token, Token::Quoted(_))
            && [
                "=", "!=", ">", "<", ">=", "<=", "=~", "!~", "^", "$", "AND", "OR", "CONTAINS",
                "REGEXP", "IN",
            ]
            .iter()
            .any(|s| token.value().eq_ignore_ascii_case(s))
    }) || tokens
        .windows(2)
        .any(|w| w[0].value().eq_ignore_ascii_case("text") && w[1].value() == "(");
    if !structured {
        let (term, limit) = if let Some(index) = tokens
            .iter()
            .position(|t| matches!(t,Token::Symbol(s) if s=="|"))
        {
            if tokens.len() != index + 3 || !tokens[index + 1].value().eq_ignore_ascii_case("LIMIT")
            {
                bail!("LIMIT must be the final search modifier");
            }
            let limit = tokens[index + 2]
                .value()
                .parse::<usize>()
                .context("LIMIT must be a positive integer")?;
            if limit == 0 {
                bail!("LIMIT must be positive");
            }
            (&input[..input.rfind('|').unwrap()].trim(), Some(limit))
        } else {
            (&input.trim(), None)
        };
        return Ok((
            if term.is_empty() {
                Value::Null
            } else {
                condition(json!({"searchScope":"global"}), "CONTAINS", term)
            },
            limit,
        ));
    }
    let mut parser = Parser {
        tokens,
        position: 0,
        signal,
        fields: available,
    };
    let query = if parser.tokens.is_empty() || parser.at("|") {
        Value::Null
    } else {
        parser.expression()?
    };
    let limit = if parser.consume("|") {
        parser.expect("LIMIT")?;
        let n = parser
            .next()?
            .value()
            .parse::<usize>()
            .context("LIMIT must be a positive integer")?;
        if n == 0 {
            bail!("LIMIT must be positive");
        }
        Some(n)
    } else {
        None
    };
    if parser.position != parser.tokens.len() {
        bail!(
            "Unexpected {}. Join conditions with AND or OR.",
            parser.tokens[parser.position].value()
        );
    }
    Ok((query, limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_wire_values() {
        assert_eq!(ns(&json!("18446744073709551615")), 18446744073709551615);
        assert_eq!(
            attribute(&json!({"kind":"int64","value":"9223372036854775807"})),
            "9223372036854775807"
        );
        assert!(
            number(&json!("0x8000000000000000"))
                .unwrap()
                .is_sign_negative()
        );
        assert_eq!(
            attribute(&json!({"kind":"double","value":"0x8000000000000000"})),
            "-0"
        );
    }
    #[test]
    fn parser_precedence_and_exact_literals() {
        let(q,l)=parse_search_request("name = 'checkout' OR statusCode = Error AND attr(span, \"count\", int64) >= 9223372036854775807 | LIMIT 50","traces").unwrap();
        assert_eq!(l, Some(50));
        assert_eq!(q["group"]["logicalOperator"], "OR");
        assert_eq!(
            q["group"]["children"][1]["group"]["children"][1]["query"]["value"],
            "9223372036854775807"
        );
    }
    #[test]
    fn search_matches_web_duration_integer_unicode_and_free_text_contracts() {
        assert_eq!(
            parse_search("duration >= 1.5ms", "traces").unwrap()["query"]["value"],
            "1500000"
        );
        assert_eq!(
            parse_search("duration IN [1s,500ms]", "traces").unwrap()["query"]["value"],
            "[\"1000000000\",\"500000000\"]"
        );
        assert_eq!(
            parse_search("statusCodeValue = 2e0", "traces").unwrap()["query"]["value"],
            "2"
        );
        assert!(parse_search("attr(span, 'x', int64) = 9223372036854775808", "traces").is_err());
        assert!(parse_search("statusCodeValue = 2.5", "traces").is_err());
        assert!(parse_search("name IN []", "traces").is_err());
        assert_eq!(
            parse_search(r#"name = "\u65e5\ud83c\udf31""#, "traces").unwrap()["query"]["value"],
            "日🌱"
        );
        assert_eq!(
            parse_search_request("payment declined (retry) | LIMIT 7", "logs")
                .unwrap()
                .0["query"]["value"],
            "payment declined (retry)"
        );
    }
    #[test]
    fn discovered_shorthand_requires_unambiguous_owner_and_type() {
        let fields = vec![
            json!({"name":"http.status_code","type":"int64","searchScope":"attribute","attributeScope":"span"}),
        ];
        assert_eq!(
            parse_search_request_with_fields("http.status_code = 402", "traces", &fields)
                .unwrap()
                .0["query"]["field"]["attributeScope"],
            "span"
        );
        let mut ambiguous = fields.clone();
        ambiguous.push(json!({"name":"http.status_code","type":"string","searchScope":"attribute","attributeScope":"resource"}));
        assert!(
            parse_search_request_with_fields("http.status_code = 402", "traces", &ambiguous)
                .is_err()
        );
        assert_eq!(
            parse_search("service.name = api-gateway", "traces").unwrap()["query"]["field"]["attributeScope"],
            "resource"
        );
    }
    #[test]
    fn quoted_null_is_not_absent() {
        assert_eq!(
            parse_search("name = 'NULL'", "traces").unwrap()["query"]["fieldOperator"],
            "="
        );
        assert_eq!(
            parse_search("name = NULL", "traces").unwrap()["query"]["fieldOperator"],
            "IS NULL"
        );
        assert!(parse_search("name =", "traces").is_err());
        assert!(parse_search("name = 'a' rubbish", "traces").is_err());
    }
}

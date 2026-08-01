//! SQX-like databank ranking / expression filters.
//!
//! Grammar (case-insensitive keywords):
//! ```text
//! expr     := or_expr
//! or_expr  := and_expr (OR and_expr)*
//! and_expr := unary (AND unary)*
//! unary    := NOT unary | '(' expr ')' | comparison
//! comparison := IDENT OP value
//! OP := > >= < <= == != =
//! value := number | 'string' | "string" | true | false | null
//! ```
//!
//! Column aliases mirror SQX Results databank names plus QuantForge EliteRow fields.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use thiserror::Error;

pub const DATABANK_FILTER_PROTOCOL: &str = "databank-filter-v1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("empty filter expression")]
    Empty,
    #[error("parse error at {position}: {message}")]
    Parse { position: usize, message: String },
    #[error("unknown column `{0}`")]
    UnknownColumn(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterExpr {
    And(Vec<FilterExpr>),
    Or(Vec<FilterExpr>),
    Not(Box<FilterExpr>),
    Compare {
        column: String,
        op: CompareOp,
        value: FilterValue,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterReport {
    pub protocol: String,
    pub expression: String,
    pub matched: usize,
    pub total: usize,
    pub fingerprints: Vec<String>,
}

/// Parse an SQX-like filter expression.
pub fn parse_filter(source: &str) -> Result<FilterExpr, FilterError> {
    let mut parser = Parser::new(source);
    let expr = parser.parse_or()?;
    parser.skip_ws();
    if parser.pos < parser.chars.len() {
        return Err(FilterError::Parse {
            position: parser.pos,
            message: format!("unexpected trailing input near `{}`", parser.rest()),
        });
    }
    Ok(expr)
}

/// Evaluate a filter against a flat JSON object of databank columns.
pub fn eval_filter(expr: &FilterExpr, row: &Map<String, Value>) -> Result<bool, FilterError> {
    match expr {
        FilterExpr::And(children) => {
            for child in children {
                if !eval_filter(child, row)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        FilterExpr::Or(children) => {
            for child in children {
                if eval_filter(child, row)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        FilterExpr::Not(child) => Ok(!eval_filter(child, row)?),
        FilterExpr::Compare { column, op, value } => {
            let key = canonicalize_column(column)?;
            let left = row.get(&key).cloned().unwrap_or(Value::Null);
            Ok(compare_values(&left, *op, value))
        }
    }
}

/// Filter rows that expose a `fingerprint` string field.
pub fn filter_rows(
    expression: &str,
    rows: &[Map<String, Value>],
) -> Result<FilterReport, FilterError> {
    let expr = parse_filter(expression)?;
    let mut fingerprints = Vec::new();
    for row in rows {
        if eval_filter(&expr, row)? {
            if let Some(Value::String(fp)) = row.get("fingerprint") {
                fingerprints.push(fp.clone());
            } else if let Some(Value::String(fp)) = row.get("strategyId") {
                fingerprints.push(fp.clone());
            }
        }
    }
    Ok(FilterReport {
        protocol: DATABANK_FILTER_PROTOCOL.into(),
        expression: expression.trim().into(),
        matched: fingerprints.len(),
        total: rows.len(),
        fingerprints,
    })
}

/// Convert an Elite-like serde value into a filterable map (accepts camelCase or snake_case).
pub fn row_from_value(value: &Value) -> Result<Map<String, Value>, FilterError> {
    let Value::Object(map) = value else {
        return Err(FilterError::Parse {
            position: 0,
            message: "row must be a JSON object".into(),
        });
    };
    let mut out = Map::new();
    for (key, val) in map {
        if let Ok(canon) = canonicalize_column(key) {
            out.insert(canon, val.clone());
        } else {
            out.insert(key.clone(), val.clone());
        }
    }
    // Promote nested metrics if present.
    if let Some(Value::Object(metrics)) = map.get("metrics") {
        for (key, val) in metrics {
            if let Ok(canon) = canonicalize_column(key) {
                out.entry(canon).or_insert_with(|| val.clone());
            }
        }
    }
    Ok(out)
}

fn canonicalize_column(raw: &str) -> Result<String, FilterError> {
    let key = raw.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
    let mapped = match key.as_str() {
        "fingerprint" => "fingerprint",
        "strategyid" | "id" | "name" => "strategyId",
        "entryconditions" | "entryconds" | "conditions" => "entryConditions",
        "exitconditions" | "exitconds" => "exitConditions",
        "islandid" | "island" => "islandId",
        "entryorder" | "ordertype" => "entryOrder",
        "management" | "mgmt" => "management",
        "evidence" | "fitness" | "score" => "evidence",
        "novelty" => "novelty",
        "trades" | "numberoftrades" | "#oftrades" | "trade_count" | "tradecount" => "trades",
        "returnpercent" | "return" | "netprofitpct" | "netprofit%" | "profitpct" => "returnPercent",
        "drawdownpercent" | "drawdown" | "maxdd" | "maxdrawdown" | "dd" => "drawdownPercent",
        "recoveryfactor" | "rf" => "recoveryFactor",
        "profitfactor" | "pf" => "profitFactor",
        "sharperatio" | "sharpe" => "sharpeRatio",
        "isexpectancy" | "expectancy" => "isExpectancy",
        "oos1expectancy" => "oos1Expectancy",
        "oos1expectancyratio" | "oos1ratio" => "oos1ExpectancyRatio",
        "complexity" => "complexity",
        "generation" | "gen" => "generation",
        "grade" => "grade",
        "parity" => "parity",
        "netprofit" | "profit" => "returnPercent",
        other => {
            return Err(FilterError::UnknownColumn(other.to_string()));
        }
    };
    Ok(mapped.into())
}

fn compare_values(left: &Value, op: CompareOp, right: &FilterValue) -> bool {
    match (left, right) {
        (Value::Null, FilterValue::Null) => matches!(op, CompareOp::Eq),
        (Value::Null, _) => matches!(op, CompareOp::Ne),
        (_, FilterValue::Null) => matches!(op, CompareOp::Ne),
        (Value::Bool(l), FilterValue::Bool(r)) => cmp_ord(&(*l as u8), &(*r as u8), op),
        (Value::Number(l), FilterValue::Number(r)) => {
            let Some(lf) = l.as_f64() else {
                return false;
            };
            cmp_f64(lf, *r, op)
        }
        (Value::String(l), FilterValue::String(r)) => {
            let (l, r) = (l.to_ascii_lowercase(), r.to_ascii_lowercase());
            match op {
                CompareOp::Eq => l == r,
                CompareOp::Ne => l != r,
                CompareOp::Gt => l > r,
                CompareOp::Ge => l >= r,
                CompareOp::Lt => l < r,
                CompareOp::Le => l <= r,
            }
        }
        (Value::String(l), FilterValue::Number(r)) => l
            .parse::<f64>()
            .map(|lf| cmp_f64(lf, *r, op))
            .unwrap_or(false),
        (Value::Number(l), FilterValue::String(r)) => r
            .parse::<f64>()
            .ok()
            .and_then(|rf| l.as_f64().map(|lf| cmp_f64(lf, rf, op)))
            .unwrap_or(false),
        _ => false,
    }
}

fn cmp_f64(left: f64, right: f64, op: CompareOp) -> bool {
    match op {
        CompareOp::Eq => (left - right).abs() <= 1e-12,
        CompareOp::Ne => (left - right).abs() > 1e-12,
        CompareOp::Gt => left > right,
        CompareOp::Ge => left >= right,
        CompareOp::Lt => left < right,
        CompareOp::Le => left <= right,
    }
}

fn cmp_ord<T: PartialOrd>(left: &T, right: &T, op: CompareOp) -> bool {
    match op {
        CompareOp::Eq => left == right,
        CompareOp::Ne => left != right,
        CompareOp::Gt => left > right,
        CompareOp::Ge => left >= right,
        CompareOp::Lt => left < right,
        CompareOp::Le => left <= right,
    }
}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    _src: &'a str,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            chars: src.chars().collect(),
            pos: 0,
            _src: src,
        }
    }

    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn parse_or(&mut self) -> Result<FilterExpr, FilterError> {
        let mut nodes = vec![self.parse_and()?];
        loop {
            self.skip_ws();
            if self.consume_keyword("OR") {
                nodes.push(self.parse_and()?);
            } else {
                break;
            }
        }
        Ok(if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            FilterExpr::Or(nodes)
        })
    }

    fn parse_and(&mut self) -> Result<FilterExpr, FilterError> {
        let mut nodes = vec![self.parse_unary()?];
        loop {
            self.skip_ws();
            if self.consume_keyword("AND") {
                nodes.push(self.parse_unary()?);
            } else {
                break;
            }
        }
        Ok(if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            FilterExpr::And(nodes)
        })
    }

    fn parse_unary(&mut self) -> Result<FilterExpr, FilterError> {
        self.skip_ws();
        if self.chars.is_empty() {
            return Err(FilterError::Empty);
        }
        if self.consume_keyword("NOT") {
            return Ok(FilterExpr::Not(Box::new(self.parse_unary()?)));
        }
        if self.consume_char('(') {
            let inner = self.parse_or()?;
            self.skip_ws();
            if !self.consume_char(')') {
                return Err(FilterError::Parse {
                    position: self.pos,
                    message: "expected `)`".into(),
                });
            }
            return Ok(inner);
        }
        self.parse_compare()
    }

    fn parse_compare(&mut self) -> Result<FilterExpr, FilterError> {
        self.skip_ws();
        let column = self.parse_ident()?;
        self.skip_ws();
        let op = self.parse_op()?;
        self.skip_ws();
        let value = self.parse_value()?;
        let _ = canonicalize_column(&column)?;
        Ok(FilterExpr::Compare { column, op, value })
    }

    fn parse_ident(&mut self) -> Result<String, FilterError> {
        let start = self.pos;
        if self.pos >= self.chars.len()
            || !(self.chars[self.pos].is_ascii_alphabetic()
                || self.chars[self.pos] == '_'
                || self.chars[self.pos] == '#')
        {
            return Err(FilterError::Parse {
                position: self.pos,
                message: "expected column name".into(),
            });
        }
        self.pos += 1;
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_alphanumeric()
                || matches!(self.chars[self.pos], '_' | '%' | '#'))
        {
            self.pos += 1;
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn parse_op(&mut self) -> Result<CompareOp, FilterError> {
        let ops = [
            (">=", CompareOp::Ge),
            ("<=", CompareOp::Le),
            ("!=", CompareOp::Ne),
            ("==", CompareOp::Eq),
            ("=", CompareOp::Eq),
            (">", CompareOp::Gt),
            ("<", CompareOp::Lt),
        ];
        for (token, op) in ops {
            if self.consume_str(token) {
                return Ok(op);
            }
        }
        Err(FilterError::Parse {
            position: self.pos,
            message: "expected comparison operator".into(),
        })
    }

    fn parse_value(&mut self) -> Result<FilterValue, FilterError> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Err(FilterError::Parse {
                position: self.pos,
                message: "expected value".into(),
            });
        }
        let ch = self.chars[self.pos];
        if ch == '\'' || ch == '"' {
            return self.parse_string(ch);
        }
        if self.consume_keyword("TRUE") {
            return Ok(FilterValue::Bool(true));
        }
        if self.consume_keyword("FALSE") {
            return Ok(FilterValue::Bool(false));
        }
        if self.consume_keyword("NULL") {
            return Ok(FilterValue::Null);
        }
        let start = self.pos;
        if ch == '-' || ch == '+' {
            self.pos += 1;
        }
        let mut saw_digit = false;
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_digit() || self.chars[self.pos] == '.')
        {
            saw_digit = true;
            self.pos += 1;
        }
        if !saw_digit {
            return Err(FilterError::Parse {
                position: start,
                message: "expected number or string".into(),
            });
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        let number: f64 = text.parse().map_err(|_| FilterError::Parse {
            position: start,
            message: format!("invalid number `{text}`"),
        })?;
        Ok(FilterValue::Number(number))
    }

    fn parse_string(&mut self, quote: char) -> Result<FilterValue, FilterError> {
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != quote {
            self.pos += 1;
        }
        if self.pos >= self.chars.len() {
            return Err(FilterError::Parse {
                position: start,
                message: "unterminated string".into(),
            });
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        self.pos += 1;
        Ok(FilterValue::String(text))
    }

    fn consume_keyword(&mut self, keyword: &str) -> bool {
        self.skip_ws();
        let end = self.pos + keyword.len();
        if end > self.chars.len() {
            return false;
        }
        let slice: String = self.chars[self.pos..end].iter().collect();
        if !slice.eq_ignore_ascii_case(keyword) {
            return false;
        }
        let boundary_ok = end == self.chars.len()
            || !(self.chars[end].is_ascii_alphanumeric() || self.chars[end] == '_');
        if !boundary_ok {
            return false;
        }
        self.pos = end;
        true
    }

    fn consume_str(&mut self, token: &str) -> bool {
        let end = self.pos + token.len();
        if end > self.chars.len() {
            return false;
        }
        let slice: String = self.chars[self.pos..end].iter().collect();
        if slice == token {
            self.pos = end;
            true
        } else {
            false
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.pos < self.chars.len() && self.chars[self.pos] == expected {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

/// Stable column help for CLI / UI.
pub fn known_columns() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("fingerprint", "Strategy structural fingerprint"),
        ("strategyId", "Strategy id / name"),
        ("trades", "Trade count"),
        ("returnPercent", "Return % (also: NetProfit, Return)"),
        ("drawdownPercent", "Max drawdown % (also: Drawdown, MaxDD)"),
        ("profitFactor", "Profit factor (also: PF)"),
        ("sharpeRatio", "Sharpe ratio"),
        ("recoveryFactor", "Recovery factor"),
        ("evidence", "Fitness / evidence score"),
        ("entryConditions", "Entry condition count"),
        ("grade", "Strategy grade string"),
        ("islandId", "MAP-Elites island id"),
        ("entryOrder", "market|stop|limit|stop_limit"),
        ("management", "Management gene summary"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(obj: Value) -> Map<String, Value> {
        row_from_value(&obj).unwrap()
    }

    #[test]
    fn parses_and_filters_sqx_style_expression() {
        let expr = parse_filter("ProfitFactor > 1.5 AND Drawdown < 20 AND Trades >= 30").unwrap();
        let pass = row(json!({
            "fingerprint": "a",
            "profitFactor": 2.0,
            "drawdownPercent": 12.0,
            "trades": 40
        }));
        let fail = row(json!({
            "fingerprint": "b",
            "profitFactor": 1.1,
            "drawdownPercent": 12.0,
            "trades": 40
        }));
        assert!(eval_filter(&expr, &pass).unwrap());
        assert!(!eval_filter(&expr, &fail).unwrap());
    }

    #[test]
    fn supports_or_not_and_strings() {
        let expr = parse_filter("grade == 'certified' OR NOT (Drawdown > 30)").unwrap();
        let row = row(json!({ "fingerprint": "c", "grade": "research", "drawdownPercent": 10.0 }));
        assert!(eval_filter(&expr, &row).unwrap());
    }

    #[test]
    fn filter_rows_reports_matches() {
        let rows = vec![
            row(json!({"fingerprint":"a","trades":10,"returnPercent":5.0})),
            row(json!({"fingerprint":"b","trades":2,"returnPercent":50.0})),
        ];
        let report = filter_rows("Trades >= 5 AND Return > 0", &rows).unwrap();
        assert_eq!(report.matched, 1);
        assert_eq!(report.fingerprints, vec!["a".to_string()]);
    }
}

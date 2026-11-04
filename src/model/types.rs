//! Cell values and column types.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};

/// Logical type of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// No values, or only nulls.
    Null,
    /// Boolean values.
    Bool,
    /// Signed 64-bit integers.
    Int,
    /// Floating-point values.
    Float,
    /// Exact decimal values preserved as text.
    Decimal,
    /// Free text (the fallback for mixed values).
    Text,
    /// Calendar dates.
    Date,
    /// Date and time without a timezone.
    DateTime,
    /// Time of day without a date.
    Time,
    /// A cell that failed to parse, or an Excel error value such as `#REF!`.
    Error,
}

impl ColumnType {
    /// Stable lowercase name used in diagnostics and reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Int => "int",
            Self::Float => "float",
            Self::Decimal => "decimal",
            Self::Text => "text",
            Self::Date => "date",
            Self::DateTime => "datetime",
            Self::Time => "time",
            Self::Error => "error",
        }
    }

    /// Combine two observed types into the narrowest type that can hold both.
    pub const fn merge(self, other: Self) -> Self {
        use ColumnType::{Decimal, Float, Int, Null, Text};

        if matches!(self, Null) {
            return other;
        }
        if matches!(other, Null) {
            return self;
        }
        if matches!(
            (self, other),
            (Int, Int) | (Float, Float) | (Decimal, Decimal)
        ) {
            return self;
        }
        if matches!(
            (self, other),
            (ColumnType::Bool, ColumnType::Bool)
                | (Text, Text)
                | (ColumnType::Date, ColumnType::Date)
                | (ColumnType::DateTime, ColumnType::DateTime)
                | (ColumnType::Time, ColumnType::Time)
                | (ColumnType::Error, ColumnType::Error)
        ) {
            return self;
        }

        match (self, other) {
            (Int, Float) | (Float, Int) => Float,
            (Int, Decimal) | (Decimal, Int) => Decimal,
            (Float, Decimal) | (Decimal, Float) => Decimal,
            _ => Text,
        }
    }
}

/// A single cell value.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    /// Exact decimal text, preserved as written.
    Decimal(String),
    Text(String),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
    Time(NaiveTime),
    /// Parse errors and spreadsheet error values (`#REF!`, `#N/A`, ...).
    Error(String),
}

impl CellValue {
    /// The logical type contributed by this value.
    pub const fn type_of(&self) -> ColumnType {
        match self {
            Self::Null => ColumnType::Null,
            Self::Bool(_) => ColumnType::Bool,
            Self::Int(_) => ColumnType::Int,
            Self::Float(_) => ColumnType::Float,
            Self::Decimal(_) => ColumnType::Decimal,
            Self::Text(_) => ColumnType::Text,
            Self::Date(_) => ColumnType::Date,
            Self::DateTime(_) => ColumnType::DateTime,
            Self::Time(_) => ColumnType::Time,
            Self::Error(_) => ColumnType::Error,
        }
    }

    /// Whether this cell is null.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Render the value as plain text for delimited output.
    ///
    /// `null` controls how [`CellValue::Null`] is written. Other values use
    /// ISO-8601 for dates/times and the shortest round-trippable form for
    /// numbers.
    pub fn to_text(&self, null: &str) -> String {
        match self {
            Self::Null => null.to_owned(),
            Self::Bool(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Decimal(value) | Self::Text(value) | Self::Error(value) => value.clone(),
            Self::Date(value) => value.format("%Y-%m-%d").to_string(),
            Self::DateTime(value) => {
                let mut rendered = value.format("%Y-%m-%dT%H:%M:%S").to_string();
                append_fraction(&mut rendered, value.and_utc().timestamp_subsec_nanos());
                rendered
            }
            Self::Time(value) => {
                let mut rendered = value.format("%H:%M:%S").to_string();
                append_fraction(&mut rendered, value.nanosecond());
                rendered
            }
        }
    }
}

fn append_fraction(rendered: &mut String, nanos: u32) {
    if nanos > 0 {
        let fraction = format!("{nanos:09}");
        rendered.push('.');
        rendered.push_str(fraction.trim_end_matches('0'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn types_merge_to_a_value_that_can_hold_both() {
        use ColumnType::*;
        assert_eq!(Null.merge(Int), Int);
        assert_eq!(Int.merge(Null), Int);
        assert_eq!(Int.merge(Float), Float);
        assert_eq!(Int.merge(Decimal), Decimal);
        assert_eq!(Bool.merge(Int), Text);
        assert_eq!(Date.merge(DateTime), Text);
    }

    #[test]
    fn values_render_as_plain_text() {
        assert_eq!(CellValue::Null.to_text("NULL"), "NULL");
        assert_eq!(CellValue::Bool(true).to_text(""), "true");
        assert_eq!(CellValue::Int(-3).to_text(""), "-3");
        assert_eq!(CellValue::Float(1.5).to_text(""), "1.5");
        assert_eq!(CellValue::Text("a,b".into()).to_text(""), "a,b");
        assert_eq!(
            CellValue::Date(NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()).to_text(""),
            "2026-11-04"
        );
    }
}

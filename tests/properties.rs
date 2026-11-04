//! Phase 11 property-style tests.
//!
//! `proptest` was not available in the offline registry used by this build, so
//! these tests use a small deterministic xorshift generator. The properties are
//! the important part: every run explores the same reproducible corpus, and a
//! failure prints the exact generated input.

use tabmux::formats::FormatId;
use tabmux::formats::csv::{self, LineEnding, ReadOptions, WriteOptions};
use tabmux::model::{CellValue, Column, ColumnType, SourceRef, Table};
use tabmux::ops::schema::infer_text_types;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, upper: usize) -> usize {
        (self.next() % upper as u64) as usize
    }
}

fn source() -> SourceRef {
    SourceRef::new("generated.csv", FormatId::Csv)
}

const TEXT_ALPHABET: &[&str] = &[
    "a", "Z", "0", "9", " ", ",", "\"", "\n", "\r\n", "\t", ";", "|", "é", "表", "\\", "'", "=",
    "-", "+", "@", "NULL",
];

fn random_text(rng: &mut Rng) -> String {
    let length = rng.below(18);
    let mut text = String::new();
    for _ in 0..length {
        text.push_str(TEXT_ALPHABET[rng.below(TEXT_ALPHABET.len())]);
    }
    text
}

fn generated_table(rng: &mut Rng, columns: usize, rows: usize) -> Table {
    let columns = (0..columns)
        .map(|index| Column::new(format!("col_{index}"), ColumnType::Text))
        .collect::<Vec<_>>();
    let rows = (0..rows)
        .map(|_| {
            (0..columns.len())
                .map(|_| CellValue::Text(random_text(rng)))
                .collect()
        })
        .collect::<Vec<_>>();
    Table::with_columns("generated", source(), columns, rows)
}

#[test]
fn csv_round_trip_preserves_generated_text() {
    let mut rng = Rng::new(0x5eed_1234_abcd_9876);

    for iteration in 0..200 {
        let columns = rng.below(6) + 1;
        let rows = rng.below(8);
        let table = generated_table(&mut rng, columns, rows);

        for (delimiter, is_tsv, line_ending) in [
            (b',', false, LineEnding::Lf),
            (b'\t', true, LineEnding::Lf),
            (b';', false, LineEnding::Crlf),
        ] {
            let mut bytes = Vec::new();
            let options = WriteOptions {
                delimiter,
                has_header: true,
                line_ending,
                null: String::new(),
            };
            csv::write(&mut bytes, &table, &options).unwrap();

            let reparsed = csv::read(&source(), &bytes, is_tsv, &ReadOptions::default())
                .unwrap_or_else(|error| {
                    panic!(
                        "iteration {iteration} delimiter {delimiter:?} failed to re-read: {error}\nbytes: {bytes:?}"
                    )
                });

            assert_eq!(reparsed.column_names(), table.column_names());
            assert_eq!(reparsed.rows, table.rows, "iteration {iteration}");
        }
    }
}

#[test]
fn empty_generated_tables_round_trip_without_a_stray_record() {
    let table = Table::with_columns("empty", source(), Vec::new(), Vec::new());
    let mut bytes = Vec::new();
    csv::write(&mut bytes, &table, &WriteOptions::new(b',')).unwrap();
    assert!(bytes.is_empty());

    let reparsed =
        csv::read(&source(), &bytes, false, &ReadOptions::default()).expect("empty output re-read");
    assert_eq!(reparsed.width(), 0);
    assert_eq!(reparsed.row_count(), 0);
}

const TYPE_CORPUS: &[&str] = &[
    "",
    "0",
    "-1",
    "007",
    "1.5",
    "-2.25",
    "1e3",
    "NaN",
    "true",
    "false",
    "TRUE",
    "2026-01-15",
    "2026-01-15T10:30:00",
    "10:30:00",
    "hello",
    "é",
    "1,5",
    "=SUM(A1:A2)",
    " ",
];

#[test]
fn text_type_inference_is_conservative_and_idempotent() {
    let mut rng = Rng::new(0xc0ff_ee00_1234_5678);

    for iteration in 0..500 {
        let value = if rng.below(2) == 0 {
            TYPE_CORPUS[rng.below(TYPE_CORPUS.len())].to_owned()
        } else {
            random_text(&mut rng)
        };

        let mut table = Table::with_columns(
            "generated",
            source(),
            vec![Column::new("value", ColumnType::Text)],
            vec![vec![CellValue::Text(value.clone())]],
        );

        infer_text_types(&mut table);
        let first_type = table.columns[0].ty;
        let first_cell = table.rows[0][0].clone();

        // Running inference again must not change the result: conversion is a
        // one-way normalisation, not a lossy per-call reinterpretation.
        infer_text_types(&mut table);
        assert_eq!(table.columns[0].ty, first_type, "value {value:?}");
        assert_eq!(table.rows[0][0], first_cell, "value {value:?}");

        match (&first_cell, first_type) {
            (CellValue::Null, _) => assert_eq!(value, "", "only empty text becomes null"),
            (CellValue::Text(text), ColumnType::Text) => assert_eq!(text, &value),
            (CellValue::Int(_), ColumnType::Int) => {}
            (CellValue::Float(_), ColumnType::Float) => {}
            (CellValue::Bool(_), ColumnType::Bool) => {}
            (CellValue::Date(_), ColumnType::Date) => {}
            (CellValue::DateTime(_), ColumnType::DateTime) => {}
            (CellValue::Time(_), ColumnType::Time) => {}
            (cell, ty) => panic!(
                "iteration {iteration}: unexpected inference {cell:?} -> {ty:?} for {value:?}"
            ),
        }
    }
}

//! Human-readable terminal rendering shared by inspection commands and query
//! output.

use crate::model::{CellValue, Table};

/// Render an aligned terminal table. Embedded newlines in cells become
/// additional physical lines so multiline values stay readable and aligned.
pub fn render_pretty(table: &Table, has_header: bool) -> String {
    let width = table.width();
    if width == 0 {
        return String::new();
    }

    let header = table
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let rows = table
        .rows
        .iter()
        .map(|row| {
            (0..width)
                .map(|column| {
                    row.get(column)
                        .unwrap_or(&CellValue::Null)
                        .to_text("")
                        .split('\n')
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut widths = vec![1usize; width];
    if has_header {
        for (column, name) in header.iter().enumerate() {
            widths[column] = widths[column].max(name.chars().count());
        }
    }
    for row in &rows {
        for (column, lines) in row.iter().enumerate() {
            for line in lines {
                widths[column] = widths[column].max(line.chars().count());
            }
        }
    }

    let mut output = String::new();
    if has_header {
        let cells = header.iter().map(String::as_str).collect::<Vec<_>>();
        push_pretty_line(&mut output, &cells, &widths);
        let separator = widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("-+-");
        output.push_str(&separator);
        output.push('\n');
    }

    for row in &rows {
        let height = row.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let cells = row
                .iter()
                .map(|lines| lines.get(line_index).map(String::as_str).unwrap_or(""))
                .collect::<Vec<_>>();
            push_pretty_line(&mut output, &cells, &widths);
        }
    }

    output
}

fn push_pretty_line(output: &mut String, cells: &[&str], widths: &[usize]) {
    let mut line = String::new();
    for (column, cell) in cells.iter().enumerate() {
        if column > 0 {
            line.push_str(" | ");
        }
        line.push_str(cell);
        let padding = widths[column].saturating_sub(cell.chars().count());
        line.push_str(&" ".repeat(padding));
    }
    output.push_str(line.trim_end());
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use crate::model::{Column, ColumnType, SourceRef};

    fn table() -> Table {
        Table::with_columns(
            "data",
            SourceRef::new("data.csv", FormatId::Csv),
            vec![
                Column::new("name", ColumnType::Text),
                Column::new("note", ColumnType::Text),
            ],
            vec![
                vec![CellValue::Text("a".into()), CellValue::Text("one".into())],
                vec![
                    CellValue::Text("b".into()),
                    CellValue::Text("two\nlines".into()),
                ],
            ],
        )
    }

    #[test]
    fn pretty_rendering_aligns_columns_and_expands_multiline_cells() {
        let rendered = render_pretty(&table(), true);
        let lines = rendered.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "name | note");
        assert_eq!(lines[1], "-----+------");
        assert_eq!(lines[2], "a    | one");
        assert_eq!(lines[3], "b    | two");
        assert_eq!(lines[4], "     | lines");
    }
}

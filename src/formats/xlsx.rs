//! XLSX/XLS/XLSB workbook reader built on `calamine`.
//!
//! Values are exported from the cached cell values. Formulas are never
//! recalculated; merged cells are expanded; workbook features that are not
//! converted (charts, styles, macros, pivot caches, comments, conditional
//! formatting, data validation) are reported as loss events.

use std::io::{Cursor, Read as _};

use calamine::{Data, Dimensions, ExcelDateTime, Range, Reader as _, Sheets};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

use crate::error::{Error, Result};
use crate::formats::SheetSelection;
use crate::model::{CellValue, Column, ColumnType, LossEvent, SourceRef, Table, TableSet};

/// Reader options for a spreadsheet workbook.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub has_header: bool,
    pub sheets: SheetSelection,
}

type Workbook = Sheets<Cursor<Vec<u8>>>;

/// Return the sheet names in workbook order.
pub fn sheet_names(source: &SourceRef, bytes: &[u8]) -> Result<Vec<String>> {
    let workbook = open(source, bytes)?;
    Ok(workbook.sheet_names())
}

/// Read the requested sheet or sheets into a [`TableSet`].
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<TableSet> {
    let mut workbook = open(source, bytes)?;
    let available = workbook.sheet_names();

    if available.is_empty() {
        return Err(Error::data(format!(
            "{}: workbook contains no worksheets",
            source.path.display()
        )));
    }

    let indices = select_sheets(source, &available, &options.sheets)?;
    let package = PackageScan::scan(bytes);

    let mut set = TableSet::new(source.clone());
    for index in indices {
        let name = available[index].clone();
        let table = read_sheet(&mut workbook, source, &name, options.has_header)?;
        set.push(table);
    }

    if let Some(first) = set.tables.first_mut() {
        for warning in package.into_warnings(source) {
            first.push_warning(warning);
        }
    }

    Ok(set)
}

fn open(source: &SourceRef, bytes: &[u8]) -> Result<Workbook> {
    calamine::open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())).map_err(|error| {
        Error::data(format!(
            "{}: cannot open workbook: {error}",
            source.path.display()
        ))
    })
}

fn select_sheets(
    source: &SourceRef,
    available: &[String],
    selection: &SheetSelection,
) -> Result<Vec<usize>> {
    match selection {
        SheetSelection::Default => Ok(vec![0]),
        SheetSelection::All => Ok((0..available.len()).collect()),
        SheetSelection::Named(name) => available
            .iter()
            .position(|sheet| sheet == name)
            .map(|index| vec![index])
            .ok_or_else(|| {
                Error::data(format!(
                    "{}: sheet '{name}' was not found; available sheets: {}",
                    source.path.display(),
                    available.join(", ")
                ))
            }),
        SheetSelection::Index(number) => {
            if *number == 0 {
                return Err(Error::usage("sheet indices are 1-based; 0 is not valid"));
            }
            if *number > available.len() {
                return Err(Error::data(format!(
                    "{}: sheet index {number} is out of range; the workbook has {} sheet(s): {}",
                    source.path.display(),
                    available.len(),
                    available.join(", ")
                )));
            }
            Ok(vec![*number - 1])
        }
    }
}

fn read_sheet(
    workbook: &mut Workbook,
    source: &SourceRef,
    name: &str,
    has_header: bool,
) -> Result<Table> {
    let sheet_source = source.clone().with_sheet(name);
    let range = workbook.worksheet_range(name).map_err(|error| {
        Error::data(format!(
            "{}: cannot read sheet '{name}': {error}",
            source.path.display()
        ))
    })?;

    let formulas = workbook
        .worksheet_formula(name)
        .ok()
        .map(formula_cells)
        .unwrap_or_default();

    let merges = match workbook {
        Sheets::Xlsx(workbook) => workbook.merge_cells_by_sheet_name(name).ok(),
        _ => None,
    };

    let start = range.start().unwrap_or((0, 0));
    let height = range.height();
    let width = range.width();

    let mut grid = vec![vec![CellValue::Null; width]; height];
    for (row_index, row) in grid.iter_mut().enumerate() {
        for (column_index, cell) in row.iter_mut().enumerate() {
            if let Some(data) = range.get((row_index, column_index)) {
                *cell = data_to_cell(data);
            }
        }
    }

    let mut warnings = Vec::new();

    if let Some(merges) = merges {
        for dimensions in merges {
            expand_merge(&mut grid, start, dimensions, &sheet_source, &mut warnings);
        }
    }

    let (top_trimmed, left_trimmed) = trim_grid(&mut grid);

    if grid.is_empty() {
        return Ok(Table::with_columns(
            name,
            sheet_source,
            Vec::new(),
            Vec::new(),
        ));
    }

    let header = if has_header {
        Some(grid.remove(0))
    } else {
        None
    };
    let width = header
        .as_ref()
        .map(Vec::len)
        .or_else(|| grid.first().map(Vec::len))
        .unwrap_or(0);

    let names = match &header {
        Some(cells) => (0..width)
            .map(|column| {
                let name = cells
                    .get(column)
                    .unwrap_or(&CellValue::Null)
                    .to_text("")
                    .trim()
                    .to_owned();
                if name.is_empty() {
                    format!("column_{}", column + 1)
                } else {
                    name
                }
            })
            .collect::<Vec<_>>(),
        None => (1..=width)
            .map(|column| format!("column_{column}"))
            .collect::<Vec<_>>(),
    };

    let mut rows = Vec::with_capacity(grid.len());
    for mut row in grid {
        row.resize(width, CellValue::Null);
        row.truncate(width);
        rows.push(row);
    }

    let mut types = vec![ColumnType::Null; width];
    for row in &rows {
        for (column, value) in row.iter().enumerate() {
            if let Some(ty) = types.get_mut(column) {
                *ty = ty.merge(value.type_of());
            }
        }
    }

    let columns = names
        .into_iter()
        .zip(types)
        .map(|(name, ty)| Column::new(name, ty))
        .collect::<Vec<_>>();

    for (position, formula) in formulas {
        let row = position.0;
        let column = position.1;
        let column_index = (column as usize).checked_sub(start.1 as usize + left_trimmed);
        let column_name = column_index
            .and_then(|index| columns.get(index))
            .map(|column| column.name.clone());
        let mut cell_source = sheet_source
            .clone()
            .with_row(row as u64 + 1)
            .with_cell(a1(row, column));
        if let Some(column_name) = column_name {
            cell_source = cell_source.with_column(column_name);
        }
        warnings.push(
            LossEvent::info(
                "excel.formula_cached_value_only",
                format!(
                    "formula at {} (`{}`) was not recalculated; the cached value was exported",
                    a1(row, column),
                    formula.trim()
                ),
            )
            .with_source(cell_source),
        );
    }

    // Keep row numbering honest even after leading blank rows were trimmed.
    let _ = top_trimmed;

    Ok(Table {
        name: name.to_owned(),
        source: sheet_source,
        columns,
        rows,
        warnings,
    })
}

fn formula_cells(range: Range<String>) -> Vec<((u32, u32), String)> {
    let Some(start) = range.start() else {
        return Vec::new();
    };

    let mut cells = Vec::new();
    for row in 0..range.height() {
        for column in 0..range.width() {
            if let Some(formula) = range.get((row, column))
                && !formula.trim().is_empty()
            {
                cells.push((
                    (start.0 + row as u32, start.1 + column as u32),
                    formula.clone(),
                ));
            }
        }
    }
    cells
}

fn data_to_cell(data: &Data) -> CellValue {
    match data {
        Data::Int(value) => CellValue::Int(*value),
        Data::Float(value) => float_to_cell(*value),
        Data::String(value) => CellValue::Text(value.clone()),
        Data::Bool(value) => CellValue::Bool(*value),
        Data::DateTime(value) => excel_date_time(value),
        Data::DateTimeIso(value) => parse_iso_date_time(value),
        Data::DurationIso(value) => CellValue::Text(value.clone()),
        Data::Error(error) => CellValue::Error(error.to_string()),
        Data::Empty => CellValue::Null,
    }
}

fn float_to_cell(value: f64) -> CellValue {
    // XLSX stores every number as a double. Present integral values as ints so
    // id/count columns do not become `1.0` in text output, while keeping real
    // fractional values as floats.
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i64::MIN as f64
        && value <= i64::MAX as f64
    {
        CellValue::Int(value as i64)
    } else {
        CellValue::Float(value)
    }
}

fn excel_date_time(value: &ExcelDateTime) -> CellValue {
    if value.is_duration() {
        return match value.as_duration() {
            Some(duration) if (0..86_400).contains(&duration.num_seconds()) => {
                let seconds = duration.num_seconds() as u32;
                match NaiveTime::from_num_seconds_from_midnight_opt(
                    seconds,
                    duration.subsec_nanos().max(0) as u32,
                ) {
                    Some(time) => CellValue::Time(time),
                    None => CellValue::Text(value.to_string()),
                }
            }
            _ => CellValue::Text(value.to_string()),
        };
    }

    match value.as_datetime() {
        Some(datetime) => datetime_to_cell(datetime),
        None => CellValue::Text(value.to_string()),
    }
}

fn parse_iso_date_time(value: &str) -> CellValue {
    if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f") {
        return datetime_to_cell(datetime);
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return datetime_to_cell(datetime);
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return CellValue::Date(date);
    }
    if let Ok(time) = NaiveTime::parse_from_str(value, "%H:%M:%S%.f") {
        return CellValue::Time(time);
    }
    CellValue::Text(value.to_owned())
}

fn datetime_to_cell(datetime: NaiveDateTime) -> CellValue {
    if datetime.time() == NaiveTime::MIN {
        CellValue::Date(datetime.date())
    } else {
        CellValue::DateTime(datetime)
    }
}

fn expand_merge(
    grid: &mut [Vec<CellValue>],
    start: (u32, u32),
    dimensions: Dimensions,
    source: &SourceRef,
    warnings: &mut Vec<LossEvent>,
) {
    if grid.is_empty() || dimensions.start.0 < start.0 || dimensions.start.1 < start.1 {
        return;
    }

    let start_row = (dimensions.start.0 - start.0) as usize;
    let start_column = (dimensions.start.1 - start.1) as usize;

    let Some(value) = grid
        .get(start_row)
        .and_then(|row| row.get(start_column))
        .cloned()
    else {
        return;
    };

    let mut expanded = false;
    for row in dimensions.start.0..=dimensions.end.0 {
        for column in dimensions.start.1..=dimensions.end.1 {
            if row < start.0 || column < start.1 {
                continue;
            }
            let relative_row = (row - start.0) as usize;
            let relative_column = (column - start.1) as usize;
            let Some(cell) = grid
                .get_mut(relative_row)
                .and_then(|row| row.get_mut(relative_column))
            else {
                continue;
            };
            if cell.is_null() {
                *cell = value.clone();
                expanded = true;
            }
        }
    }

    if expanded {
        warnings.push(
            LossEvent::info(
                "excel.merged_cell_expanded",
                format!(
                    "merged range {}:{} was expanded into all covered cells",
                    a1(dimensions.start.0, dimensions.start.1),
                    a1(dimensions.end.0, dimensions.end.1)
                ),
            )
            .with_source(
                source
                    .clone()
                    .with_row(dimensions.start.0 as u64 + 1)
                    .with_cell(a1(dimensions.start.0, dimensions.start.1)),
            ),
        );
    }
}

/// Remove leading/trailing empty rows and columns. Returns the number of
/// rows and columns removed from the start of the used range.
fn trim_grid(grid: &mut Vec<Vec<CellValue>>) -> (usize, usize) {
    let mut top = 0;
    while grid
        .first()
        .is_some_and(|row| row.iter().all(CellValue::is_null))
    {
        grid.remove(0);
        top += 1;
    }
    while grid
        .last()
        .is_some_and(|row| row.iter().all(CellValue::is_null))
    {
        grid.pop();
    }

    if grid.is_empty() {
        return (top, 0);
    }

    let mut left = 0;
    loop {
        let empty = grid
            .iter()
            .all(|row| row.get(left).is_none_or(CellValue::is_null));
        if !empty {
            break;
        }
        left += 1;
        if grid.iter().all(|row| left >= row.len()) {
            break;
        }
    }

    if left > 0 {
        for row in grid.iter_mut() {
            row.drain(0..left.min(row.len()));
        }
    }

    while grid
        .iter()
        .all(|row| row.last().is_none_or(CellValue::is_null))
    {
        for row in grid.iter_mut() {
            row.pop();
        }
        if grid.iter().all(Vec::is_empty) {
            break;
        }
    }

    (top, left)
}

fn a1(row: u32, column: u32) -> String {
    let mut number = column as u64 + 1;
    let mut letters = String::new();
    while number > 0 {
        let remainder = ((number - 1) % 26) as u8;
        letters.insert(0, (b'A' + remainder) as char);
        number = (number - 1) / 26;
    }
    format!("{letters}{}", row + 1)
}

#[derive(Debug, Default)]
struct PackageScan {
    charts: usize,
    styled_cells: bool,
    macros: usize,
    pivots: usize,
    comments: usize,
    conditional_formatting: usize,
    data_validation: usize,
}

impl PackageScan {
    fn scan(bytes: &[u8]) -> Self {
        let mut scan = Self::default();
        let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else {
            return scan;
        };

        for index in 0..archive.len() {
            let Ok(mut entry) = archive.by_index(index) else {
                continue;
            };
            let Ok(name) = entry.name() else {
                continue;
            };
            let name = name.to_string();

            if name.starts_with("xl/charts/") && name.ends_with(".xml") {
                scan.charts += 1;
                continue;
            }
            if name == "xl/vbaProject.bin" {
                scan.macros += 1;
                continue;
            }
            if name.starts_with("xl/pivotCache/") {
                scan.pivots += 1;
                continue;
            }
            if name.starts_with("xl/comments") {
                scan.comments += 1;
                continue;
            }

            let interesting = name == "xl/styles.xml" || name.starts_with("xl/worksheets/");
            if !interesting {
                continue;
            }

            let mut text = String::new();
            if entry
                .by_ref()
                .take(8 * 1024 * 1024)
                .read_to_string(&mut text)
                .is_err()
            {
                continue;
            }

            if name == "xl/styles.xml" {
                scan.styled_cells = cell_format_count(&text) > 1;
            } else {
                scan.conditional_formatting += text.matches("<conditionalFormatting").count();
                scan.data_validation += text.matches("<dataValidation").count();
            }
        }

        scan
    }

    fn into_warnings(self, source: &SourceRef) -> Vec<LossEvent> {
        let mut warnings = Vec::new();

        if self.charts > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.chart_dropped",
                    format!(
                        "the workbook contains {} chart(s); charts are not copied",
                        self.charts
                    ),
                )
                .with_source(source.clone()),
            );
        }
        if self.styled_cells {
            warnings.push(
                LossEvent::warning(
                    "excel.style_dropped",
                    "cell styles and number formats are not preserved; values are exported without formatting",
                )
                .with_source(source.clone()),
            );
        }
        if self.macros > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.macro_dropped",
                    "the workbook contains a VBA project; macros are never executed or copied",
                )
                .with_source(source.clone()),
            );
        }
        if self.pivots > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.pivot_cache_dropped",
                    format!(
                        "the workbook contains {} pivot cache part(s); pivot tables are not copied",
                        self.pivots
                    ),
                )
                .with_source(source.clone()),
            );
        }
        if self.comments > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.comment_dropped",
                    format!(
                        "the workbook contains {} comment part(s); comments are not copied",
                        self.comments
                    ),
                )
                .with_source(source.clone()),
            );
        }
        if self.conditional_formatting > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.conditional_formatting_dropped",
                    format!(
                        "{} conditional formatting rule(s) are not copied",
                        self.conditional_formatting
                    ),
                )
                .with_source(source.clone()),
            );
        }
        if self.data_validation > 0 {
            warnings.push(
                LossEvent::warning(
                    "excel.data_validation_dropped",
                    format!(
                        "{} data validation rule(s) are not copied",
                        self.data_validation
                    ),
                )
                .with_source(source.clone()),
            );
        }

        warnings
    }
}

fn cell_format_count(styles: &str) -> usize {
    let Some(index) = styles.find("<cellXfs") else {
        return 0;
    };
    let rest = &styles[index..];
    let Some(count_index) = rest.find("count=\"") else {
        return 0;
    };
    let start = count_index + "count=\"".len();
    let Some(end) = rest[start..].find('"') else {
        return 0;
    };
    rest[start..start + end].parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;
    use std::path::Path;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name)
    }

    fn source(name: &str) -> SourceRef {
        SourceRef::new(name, FormatId::Xlsx)
    }

    fn read_fixture(name: &str, sheets: SheetSelection) -> TableSet {
        let path = fixture(name);
        let bytes = std::fs::read(&path).unwrap();
        read(
            &source(name),
            &bytes,
            &ReadOptions {
                has_header: true,
                sheets,
            },
        )
        .unwrap()
    }

    #[test]
    fn lists_sheets_in_workbook_order() {
        let bytes = std::fs::read(fixture("sales.xlsx")).unwrap();
        let names = sheet_names(&source("sales.xlsx"), &bytes).unwrap();
        assert_eq!(names, ["Sales", "Regions", "Notes"]);
    }

    #[test]
    fn reads_the_selected_sheet_with_types_and_dates() {
        let set = read_fixture("sales.xlsx", SheetSelection::Named("Sales".into()));
        assert_eq!(set.len(), 1);
        let table = &set.tables[0];
        assert_eq!(
            table.column_names(),
            ["id", "name", "region", "amount", "sold_on", "note"]
        );
        assert_eq!(table.rows[0][0], CellValue::Int(1));
        assert_eq!(table.rows[0][1], CellValue::Text("Alice".into()));
        assert!(matches!(table.rows[0][4], CellValue::Date(_)));
        // The merged total row puts text in the id column, so that column's
        // inferred type is text; amount and sold_on stay numeric/date.
        assert_eq!(table.columns[3].ty, ColumnType::Float);
        assert_eq!(table.columns[4].ty, ColumnType::Date);
    }

    #[test]
    fn expands_merged_cells_and_reports_them() {
        let set = read_fixture("sales.xlsx", SheetSelection::Named("Sales".into()));
        let table = &set.tables[0];
        let total_row = table
            .rows
            .iter()
            .find(|row| row[0] == CellValue::Text("Total".into()))
            .expect("the total row is present");
        assert_eq!(total_row[1], CellValue::Text("Total".into()));
        assert!(
            table
                .warnings
                .iter()
                .any(|warning| warning.code == "excel.merged_cell_expanded")
        );
    }

    #[test]
    fn reports_formula_cached_values_and_workbook_features() {
        let set = read_fixture("sales.xlsx", SheetSelection::Named("Sales".into()));
        let codes = set.tables[0]
            .warnings
            .iter()
            .map(|warning| warning.code)
            .collect::<Vec<_>>();
        assert!(codes.contains(&"excel.formula_cached_value_only"));
        assert!(codes.contains(&"excel.chart_dropped"));
        assert!(codes.contains(&"excel.style_dropped"));
    }

    #[test]
    fn all_sheets_returns_each_table() {
        let set = read_fixture("sales.xlsx", SheetSelection::All);
        assert_eq!(set.len(), 3);
        assert_eq!(set.tables[0].name, "Sales");
        assert_eq!(set.tables[1].name, "Regions");
        assert_eq!(set.tables[2].name, "Notes");
    }

    #[test]
    fn missing_sheet_error_lists_available_sheets() {
        let path = fixture("sales.xlsx");
        let bytes = std::fs::read(path).unwrap();
        let error = read(
            &source("sales.xlsx"),
            &bytes,
            &ReadOptions {
                has_header: true,
                sheets: SheetSelection::Named("Missing".into()),
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("Missing"));
        assert!(message.contains("Sales"));
        assert!(message.contains("Regions"));
    }

    #[test]
    fn column_letters_use_a1_notation() {
        assert_eq!(a1(0, 0), "A1");
        assert_eq!(a1(4, 1), "B5");
        assert_eq!(a1(9, 25), "Z10");
        assert_eq!(a1(0, 26), "AA1");
    }
}

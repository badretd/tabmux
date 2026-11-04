//! Tolerant HTML table reader.
//!
//! The reader is intentionally dependency-free: it tokenises enough of HTML to
//! find `<table>`, `<caption>`, `<thead>`/`<tbody>`/`<tfoot>`, `<tr>`, and
//! `<td>`/`<th>` elements, then expands `colspan`/`rowspan` into a rectangular
//! grid. It never executes scripts and never fetches external resources.
//!
//! Nested tables become separate tables in document order. The outer cell keeps
//! a `[table N]` marker so the relationship is visible instead of silently
//! discarded.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::formats::{MergePolicy, SheetSelection};
use crate::io::decode_text;
use crate::model::{
    CellValue, Column, ColumnType, LossEvent, SkippedTable, SourceRef, Table, TableSet,
};

const MAX_SPAN: usize = 1_000;

/// Reader options for HTML input.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    pub encoding: Option<String>,
    pub has_header: bool,
    pub sheets: SheetSelection,
    pub merge_policy: MergePolicy,
    /// Explicit 1-based header row. Rows before it are discarded.
    pub header_row: Option<usize>,
    /// Use `<caption>` text for table names instead of `table-N`.
    pub use_captions: bool,
    /// Skip tables with fewer rows than this (after merge expansion and
    /// leading/trailing empty row trimming).
    pub min_rows: usize,
    /// Skip tables with fewer columns than this.
    pub min_cols: usize,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            encoding: None,
            has_header: true,
            sheets: SheetSelection::Default,
            merge_policy: MergePolicy::Expand,
            header_row: None,
            use_captions: false,
            min_rows: 2,
            min_cols: 1,
        }
    }
}

/// Read the selected HTML table(s) from already-loaded bytes.
pub fn read(source: &SourceRef, bytes: &[u8], options: &ReadOptions) -> Result<TableSet> {
    let decoded = decode_text(bytes, options.encoding.as_deref())?;
    let tokens = tokenize(&decoded.text);
    let parsed = Parser::new(tokens).parse_document()?;

    let mut built: Vec<Option<Table>> = Vec::with_capacity(parsed.len());
    let mut skipped: Vec<SkippedTable> = Vec::new();
    for parsed_table in parsed {
        let index = parsed_table.index;
        match build_table(source, parsed_table, options)? {
            Ok(table) => built.push(Some(table)),
            Err(skipped_table) => {
                debug_assert_eq!(skipped_table.table, index + 1);
                skipped.push(skipped_table);
                built.push(None);
            }
        }
    }

    let selected = select_tables(source, &built, &skipped, &options.sheets)?;

    let mut set = TableSet::new(source.clone());
    for index in selected {
        if let Some(table) = built[index].take() {
            set.push(table);
        }
    }

    if decoded.had_errors
        && let Some(first) = set.tables.first_mut()
    {
        first.push_warning(
            LossEvent::warning(
                "csv.encoding_replaced",
                format!(
                    "{}: invalid bytes were replaced while decoding the HTML input",
                    source.path.display()
                ),
            )
            .with_source(first.source.clone()),
        );
    }

    if !skipped.is_empty()
        && let Some(first) = set.tables.first_mut()
    {
        for skipped_table in &skipped {
            let label = skipped_table
                .caption
                .clone()
                .filter(|caption| !caption.trim().is_empty())
                .unwrap_or_else(|| format!("table-{}", skipped_table.table));
            first.push_warning(
                LossEvent::info(
                    "html.table_skipped",
                    format!(
                        "table {} ('{label}') was skipped: {}",
                        skipped_table.table, skipped_table.reason
                    ),
                )
                .with_source(skipped_table.source.clone()),
            );
        }
    }

    if set.tables.is_empty() {
        return Err(Error::data(format!(
            "{}: no HTML tables were selected ({} table(s) were skipped or below --min-rows/--min-cols)",
            source.path.display(),
            skipped.len()
        )));
    }

    set.skipped = skipped;
    assign_table_names(&mut set.tables);

    Ok(set)
}

/// List the selectable table names. Captions are preferred when present.
pub fn table_names(source: &SourceRef, bytes: &[u8]) -> Result<Vec<String>> {
    let options = ReadOptions {
        sheets: SheetSelection::All,
        use_captions: true,
        ..ReadOptions::default()
    };
    let set = read(source, bytes, &options)?;
    Ok(set.tables.into_iter().map(|table| table.name).collect())
}

fn select_tables(
    source: &SourceRef,
    built: &[Option<Table>],
    skipped: &[SkippedTable],
    selection: &SheetSelection,
) -> Result<Vec<usize>> {
    match selection {
        SheetSelection::Default => built
            .iter()
            .position(Option::is_some)
            .map(|index| vec![index])
            .ok_or_else(|| no_tables_error(source, skipped)),
        SheetSelection::All => Ok(built
            .iter()
            .enumerate()
            .filter_map(|(index, table)| table.as_ref().map(|_| index))
            .collect()),
        SheetSelection::Index(number) => {
            if *number == 0 {
                return Err(Error::usage("table indices are 1-based; 0 is not valid"));
            }
            let index = number - 1;
            if index >= built.len() {
                return Err(Error::data(format!(
                    "{}: table index {number} is out of range; the document has {} table(s)",
                    source.path.display(),
                    built.len()
                )));
            }
            if built[index].is_none() {
                let reason = skipped
                    .iter()
                    .find(|skipped| skipped.table == *number)
                    .map(|skipped| skipped.reason.as_str())
                    .unwrap_or("the table was skipped");
                return Err(Error::data(format!(
                    "{}: table {number} was skipped: {reason}",
                    source.path.display()
                )));
            }
            Ok(vec![index])
        }
        SheetSelection::Named(name) => built
            .iter()
            .enumerate()
            .find_map(|(index, table)| {
                table
                    .as_ref()
                    .filter(|table| {
                        table.name == *name
                            || table.source.caption.as_deref() == Some(name.as_str())
                            || table.source.table.as_deref() == Some(name.as_str())
                    })
                    .map(|_| index)
            })
            .map(|index| vec![index])
            .ok_or_else(|| {
                let available = built
                    .iter()
                    .filter_map(|table| table.as_ref().map(|table| table.name.as_str()))
                    .collect::<Vec<_>>();
                Error::data(format!(
                    "{}: table '{name}' was not found; available tables: {}",
                    source.path.display(),
                    available.join(", ")
                ))
            }),
    }
}

fn no_tables_error(source: &SourceRef, skipped: &[SkippedTable]) -> Error {
    if skipped.is_empty() {
        Error::data(format!("{}: no tables found", source.path.display()))
    } else {
        Error::data(format!(
            "{}: no HTML tables were selected ({} table(s) were below --min-rows/--min-cols)",
            source.path.display(),
            skipped.len()
        ))
    }
}

/// Deduplicate table names case-insensitively after captions are applied.
fn assign_table_names(tables: &mut [Table]) {
    let mut used = HashSet::new();
    for table in tables.iter_mut() {
        let original = table.name.clone();
        let mut candidate = original.clone();
        let mut suffix = 2;
        while !used.insert(candidate.to_lowercase()) {
            candidate = format!("{original}-{suffix}");
            suffix += 1;
        }
        if candidate != original {
            table.push_warning(
                LossEvent::info(
                    "html.sheet_name_collision",
                    format!("table name '{original}' collided; using '{candidate}'"),
                )
                .with_source(table.source.clone()),
            );
            table.name = candidate;
        }
    }
}

fn build_table(
    source: &SourceRef,
    parsed: ParsedTable,
    options: &ReadOptions,
) -> Result<std::result::Result<Table, SkippedTable>> {
    let number = parsed.index + 1;
    let mut table_source = source
        .clone()
        .with_table(format!("table-{number}"))
        .with_caption_opt(parsed.caption.clone());
    if let Some(parent) = parsed.nested_in.clone() {
        table_source = table_source.with_nested_in(parent);
    }

    let mut warnings = parsed.warnings.clone();
    for (row_index, row) in parsed.rows.iter().enumerate() {
        for cell in &row.cells {
            if let Some(nested) = cell.nested_table {
                warnings.push(
                    LossEvent::warning(
                        "html.nested_table_extracted",
                        format!(
                            "nested table {} was extracted as a separate table; the outer cell was replaced with the marker '[table {}]'",
                            nested + 1,
                            nested + 1
                        ),
                    )
                    .with_source(
                        source
                            .clone()
                            .with_table(format!("table-{number}"))
                            .with_row(row_index as u64 + 1),
                    ),
                );
            }
        }
    }

    let (mut grid, mut continuation, mut meta, merge_warnings, merge_events) =
        expand_grid(source, &parsed, options)?;
    warnings.extend(merge_warnings);

    let mut width = grid.iter().map(Vec::len).max().unwrap_or(0);
    let top_trimmed = trim_edges(&mut grid, &mut continuation, &mut meta, &mut width);

    if grid.is_empty() || grid.len() < options.min_rows || width < options.min_cols {
        let reason = skip_reason(grid.len(), width, options);
        return Ok(Err(SkippedTable {
            source: table_source,
            table: number,
            caption: parsed.caption.clone(),
            rows: grid.len(),
            columns: width,
            reason,
        }));
    }

    let header_index = resolve_header(&grid, &meta, options, &mut warnings, &table_source)?;

    // A horizontally merged header cell is already reported by
    // `html.header_colspan_expanded`, so do not repeat it as a generic
    // merged-cell expansion. Merges that reach into data rows still need
    // their own event.
    let header_row = header_index.map(|index| index + top_trimmed);
    for merge in merge_events {
        if merge.event.code == "html.merged_cell_expanded"
            && merge.rowspan == 1
            && Some(merge.row) == header_row
        {
            continue;
        }
        warnings.push(merge.event);
    }

    let names = match header_index {
        Some(index) => column_names(
            &grid[index],
            &continuation[index],
            width,
            &mut warnings,
            &table_source,
        ),
        None => (1..=width)
            .map(|column| format!("column_{column}"))
            .collect::<Vec<_>>(),
    };

    let data_start = header_index.map(|index| index + 1).unwrap_or(0);
    let mut rows = Vec::with_capacity(grid.len().saturating_sub(data_start));
    for mut row in grid.into_iter().skip(data_start) {
        row.resize(width, CellValue::Null);
        row.truncate(width);
        rows.push(row);
    }

    let mut types = vec![ColumnType::Null; width];
    for row in &rows {
        for (index, value) in row.iter().enumerate() {
            if let Some(ty) = types.get_mut(index) {
                *ty = ty.merge(value.type_of());
            }
        }
    }

    let columns = names
        .into_iter()
        .zip(types)
        .map(|(name, ty)| Column::new(name, ty))
        .collect::<Vec<_>>();

    let first_data_row =
        top_trimmed as u64 + header_index.map(|index| index as u64 + 2).unwrap_or(1);

    let name = if options.use_captions {
        parsed
            .caption
            .filter(|caption| !caption.trim().is_empty())
            .unwrap_or_else(|| format!("table-{number}"))
    } else {
        format!("table-{number}")
    };

    let mut table = Table::with_columns(name, table_source, columns, rows);
    table.warnings = warnings;
    table.first_data_row = first_data_row;
    Ok(Ok(table))
}

fn skip_reason(rows: usize, columns: usize, options: &ReadOptions) -> String {
    let mut reasons = Vec::new();
    if rows < options.min_rows {
        reasons.push(format!("{rows} row(s) < --min-rows {}", options.min_rows));
    }
    if columns < options.min_cols {
        reasons.push(format!(
            "{columns} column(s) < --min-cols {}",
            options.min_cols
        ));
    }
    if reasons.is_empty() {
        "the table has no rows".to_owned()
    } else {
        reasons.join("; ")
    }
}

type ExpandedGrid = (
    Vec<Vec<CellValue>>,
    Vec<Vec<bool>>,
    Vec<RowMeta>,
    Vec<LossEvent>,
    Vec<MergeEvent>,
);

/// A merged cell that was expanded or truncated. Kept aside until the header
/// row is known so header-only expansions can be de-duplicated.
struct MergeEvent {
    row: usize,
    rowspan: usize,
    event: LossEvent,
}

fn expand_grid(
    source: &SourceRef,
    parsed: &ParsedTable,
    options: &ReadOptions,
) -> Result<ExpandedGrid> {
    let number = parsed.index + 1;
    let mut grid: Vec<Vec<CellValue>> = Vec::new();
    let mut continuation: Vec<Vec<bool>> = Vec::new();
    let mut occupied: Vec<Vec<bool>> = Vec::new();
    let mut meta: Vec<RowMeta> = parsed.rows.iter().map(|row| row.meta).collect();
    let mut warnings = Vec::new();
    let mut merge_events = Vec::new();

    for (row_index, row) in parsed.rows.iter().enumerate() {
        ensure_row(
            &mut grid,
            &mut continuation,
            &mut occupied,
            &mut meta,
            row_index,
        );
        let mut column = 0usize;

        for cell in &row.cells {
            while occupied
                .get(row_index)
                .and_then(|row| row.get(column))
                .copied()
                .unwrap_or(false)
            {
                column += 1;
            }

            let colspan = cell.colspan;
            let rowspan = cell.rowspan;
            let merged = colspan > 1 || rowspan > 1;

            if merged && options.merge_policy == MergePolicy::Error {
                return Err(Error::data(format!(
                    "{}: table {number} row {} has a merged cell (colspan={colspan}, rowspan={rowspan}); use --merge-policy expand or first",
                    source.path.display(),
                    row_index + 1
                )));
            }

            for row_offset in 0..rowspan {
                let target_row = row_index + row_offset;
                ensure_row(
                    &mut grid,
                    &mut continuation,
                    &mut occupied,
                    &mut meta,
                    target_row,
                );

                for column_offset in 0..colspan {
                    let target_column = column + column_offset;
                    ensure_column(
                        &mut grid,
                        &mut continuation,
                        &mut occupied,
                        target_row,
                        target_column,
                    );

                    if occupied[target_row][target_column] {
                        warnings.push(
                            LossEvent::warning(
                                "html.merged_cell_overlap",
                                format!(
                                    "merged cell at table row {} column {} overlaps another cell; the existing value was kept",
                                    target_row + 1,
                                    target_column + 1
                                ),
                            )
                            .with_source(
                                source
                                    .clone()
                                    .with_table(format!("table-{number}"))
                                    .with_row(target_row as u64 + 1),
                            ),
                        );
                        continue;
                    }

                    occupied[target_row][target_column] = true;
                    continuation[target_row][target_column] = row_offset > 0 || column_offset > 0;
                    let value = if (row_offset == 0 && column_offset == 0)
                        || options.merge_policy == MergePolicy::Expand
                    {
                        cell_value(&cell.text)
                    } else {
                        CellValue::Null
                    };
                    grid[target_row][target_column] = value;
                }
            }

            if merged {
                let code = if options.merge_policy == MergePolicy::Expand {
                    "html.merged_cell_expanded"
                } else {
                    "html.merged_cell_first_only"
                };
                let message = if options.merge_policy == MergePolicy::Expand {
                    format!(
                        "merged cell at table row {} column {} (colspan={colspan}, rowspan={rowspan}) was expanded into all covered cells",
                        row_index + 1,
                        column + 1
                    )
                } else {
                    format!(
                        "merged cell at table row {} column {} (colspan={colspan}, rowspan={rowspan}) kept its value in the first cell only",
                        row_index + 1,
                        column + 1
                    )
                };
                merge_events.push(MergeEvent {
                    row: row_index,
                    rowspan,
                    event: LossEvent::info(code, message).with_source(
                        source
                            .clone()
                            .with_table(format!("table-{number}"))
                            .with_row(row_index as u64 + 1),
                    ),
                });
            }

            column += colspan;
        }
    }

    Ok((grid, continuation, meta, warnings, merge_events))
}

fn ensure_row(
    grid: &mut Vec<Vec<CellValue>>,
    continuation: &mut Vec<Vec<bool>>,
    occupied: &mut Vec<Vec<bool>>,
    meta: &mut Vec<RowMeta>,
    row: usize,
) {
    while grid.len() <= row {
        grid.push(Vec::new());
        continuation.push(Vec::new());
        occupied.push(Vec::new());
    }
    while meta.len() <= row {
        meta.push(RowMeta::default());
    }
}

fn ensure_column(
    grid: &mut [Vec<CellValue>],
    continuation: &mut [Vec<bool>],
    occupied: &mut [Vec<bool>],
    row: usize,
    column: usize,
) {
    while grid[row].len() <= column {
        grid[row].push(CellValue::Null);
        continuation[row].push(false);
        occupied[row].push(false);
    }
}

fn trim_edges(
    grid: &mut Vec<Vec<CellValue>>,
    continuation: &mut Vec<Vec<bool>>,
    meta: &mut Vec<RowMeta>,
    width: &mut usize,
) -> usize {
    let mut top = 0;
    while grid
        .first()
        .is_some_and(|row| row.iter().all(CellValue::is_null))
    {
        grid.remove(0);
        continuation.remove(0);
        top += 1;
        if !meta.is_empty() {
            meta.remove(0);
        }
    }
    while grid
        .last()
        .is_some_and(|row| row.iter().all(CellValue::is_null))
    {
        grid.pop();
        continuation.pop();
        meta.pop();
    }

    let max_width = grid.iter().map(Vec::len).max().unwrap_or(0);
    let mut left = 0;
    while left < max_width
        && grid
            .iter()
            .all(|row| row.get(left).is_none_or(CellValue::is_null))
    {
        left += 1;
    }
    if left > 0 {
        for row in grid.iter_mut() {
            row.drain(0..left.min(row.len()));
        }
        for row in continuation.iter_mut() {
            row.drain(0..left.min(row.len()));
        }
    }

    loop {
        let empty = grid
            .iter()
            .all(|row| row.last().is_none_or(CellValue::is_null));
        if !empty || grid.iter().all(Vec::is_empty) {
            break;
        }
        for row in grid.iter_mut() {
            row.pop();
        }
        for row in continuation.iter_mut() {
            row.pop();
        }
    }

    *width = grid.iter().map(Vec::len).max().unwrap_or(0);
    top
}

fn resolve_header(
    grid: &[Vec<CellValue>],
    meta: &[RowMeta],
    options: &ReadOptions,
    warnings: &mut Vec<LossEvent>,
    source: &SourceRef,
) -> Result<Option<usize>> {
    if !options.has_header {
        return Ok(None);
    }

    if let Some(number) = options.header_row {
        if number == 0 {
            return Err(Error::usage("--header-row is 1-based; 0 is not valid"));
        }
        if number > grid.len() {
            return Err(Error::data(format!(
                "{}: --header-row {number} is out of range; the table has {} row(s)",
                source.path.display(),
                grid.len()
            )));
        }
        return Ok(Some(number - 1));
    }

    let candidate = meta
        .iter()
        .position(|row| row.in_thead || row.has_th)
        .or_else(|| (!grid.is_empty()).then_some(0));

    if let Some(index) = candidate
        && !meta
            .get(index)
            .is_some_and(|row| row.in_thead || row.has_th)
    {
        warnings.push(
            LossEvent::info(
                "html.header_inferred",
                if index == 0 {
                    "no <th> cells were found; used the first row as the header".to_owned()
                } else {
                    format!("used row {} as the header", index + 1)
                },
            )
            .with_source(source.clone()),
        );
    }

    Ok(candidate)
}

fn column_names(
    header: &[CellValue],
    continuation: &[bool],
    width: usize,
    warnings: &mut Vec<LossEvent>,
    source: &SourceRef,
) -> Vec<String> {
    let mut used = HashSet::<String>::new();
    let mut names = Vec::with_capacity(width);

    for column in 0..width {
        let raw = header
            .get(column)
            .map(|value| value.to_text("").trim().to_owned())
            .unwrap_or_default();
        let base = if raw.is_empty() {
            format!("column_{}", column + 1)
        } else {
            raw
        };

        let mut name = base.clone();
        let mut suffix = 2usize;
        while !used.insert(name.to_lowercase()) {
            name = format!("{base}_{suffix}");
            suffix += 1;
        }

        if name != base {
            if continuation.get(column).copied().unwrap_or(false) {
                warnings.push(
                    LossEvent::info(
                        "html.header_colspan_expanded",
                        format!(
                            "header '{base}' spans multiple columns; expanded column name '{name}'"
                        ),
                    )
                    .with_source(source.clone().with_column(base)),
                );
            } else {
                warnings.push(
                    LossEvent::warning(
                        "html.duplicate_header",
                        format!("duplicate header '{base}' was renamed to '{name}'"),
                    )
                    .with_source(source.clone()),
                );
            }
        }

        names.push(name);
    }

    names
}

fn cell_value(text: &str) -> CellValue {
    if text.is_empty() {
        CellValue::Null
    } else {
        CellValue::Text(text.to_owned())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RowMeta {
    has_th: bool,
    in_thead: bool,
}

#[derive(Debug, Clone)]
struct ParsedTable {
    index: usize,
    caption: Option<String>,
    nested_in: Option<String>,
    rows: Vec<ParsedRow>,
    warnings: Vec<LossEvent>,
}

#[derive(Debug, Clone)]
struct ParsedRow {
    cells: Vec<ParsedCell>,
    meta: RowMeta,
}

#[derive(Debug, Clone)]
struct ParsedCell {
    text: String,
    colspan: usize,
    rowspan: usize,
    nested_table: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Head,
    Body,
    Foot,
}

#[derive(Debug, Clone)]
enum Token {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
    },
    End {
        name: String,
    },
    Text(String),
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    tables: Vec<Option<ParsedTable>>,
    next_index: usize,
    current_table: Option<usize>,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            position: 0,
            tables: Vec::new(),
            next_index: 0,
            current_table: None,
        }
    }

    fn parse_document(mut self) -> Result<Vec<ParsedTable>> {
        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::Start { name, .. } if name == "table" => {
                    self.parse_table()?;
                }
                _ => self.position += 1,
            }
        }

        Ok(self.tables.into_iter().flatten().collect())
    }

    fn parse_table(&mut self) -> Result<usize> {
        if matches!(
            self.tokens.get(self.position),
            Some(Token::Start { name, .. }) if name == "table"
        ) {
            self.position += 1;
        }

        let index = self.next_index;
        self.next_index += 1;
        self.tables.push(None);
        let parent = self.current_table;
        self.current_table = Some(index);

        let mut table = ParsedTable {
            index,
            caption: None,
            nested_in: None,
            rows: Vec::new(),
            warnings: Vec::new(),
        };
        let mut section = Section::None;

        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::End { name } if name == "table" => {
                    self.position += 1;
                    break;
                }
                Token::Start { name, .. } if name == "caption" => {
                    self.position += 1;
                    table.caption = Some(self.parse_caption());
                }
                Token::Start { name, .. } if name == "thead" => {
                    section = Section::Head;
                    self.position += 1;
                }
                Token::Start { name, .. } if name == "tbody" => {
                    section = Section::Body;
                    self.position += 1;
                }
                Token::Start { name, .. } if name == "tfoot" => {
                    section = Section::Foot;
                    self.position += 1;
                }
                Token::End { name } if name == "thead" || name == "tbody" || name == "tfoot" => {
                    section = Section::None;
                    self.position += 1;
                }
                Token::Start { name, .. } if name == "tr" => {
                    let row = self.parse_row(section);
                    table.rows.push(row);
                }
                Token::Start { name, .. } if name == "td" || name == "th" => {
                    // Tolerate a row without an explicit `<tr>`.
                    let row = self.parse_row(section);
                    table.rows.push(row);
                }
                Token::Start { name, .. } if name == "table" => {
                    // A table outside any cell is still a separate table.
                    self.parse_table()?;
                }
                _ => self.position += 1,
            }
        }

        self.tables[index] = Some(table);
        self.current_table = parent;
        Ok(index)
    }

    fn parse_caption(&mut self) -> String {
        let mut raw = String::new();

        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::End { name } if name == "caption" => {
                    self.position += 1;
                    break;
                }
                Token::End { name } if name == "table" => break,
                Token::Text(text) => {
                    raw.push_str(text);
                    self.position += 1;
                }
                Token::Start { name, .. } if name == "br" => {
                    raw.push('\u{0}');
                    self.position += 1;
                }
                _ => self.position += 1,
            }
        }

        normalize_text(&raw)
    }

    fn parse_row(&mut self, section: Section) -> ParsedRow {
        if matches!(
            self.tokens.get(self.position),
            Some(Token::Start { name, .. }) if name == "tr"
        ) {
            self.position += 1;
        }

        let mut cells = Vec::new();
        let mut has_th = false;

        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::End { name } if name == "tr" => {
                    self.position += 1;
                    break;
                }
                Token::End { name } if name == "table" => break,
                Token::Start { name, .. } if name == "tr" => break,
                Token::Start { name, .. } if name == "td" || name == "th" => {
                    let header = name == "th";
                    has_th |= header;
                    cells.push(self.parse_cell(header));
                }
                _ => self.position += 1,
            }
        }

        ParsedRow {
            cells,
            meta: RowMeta {
                has_th,
                in_thead: section == Section::Head,
            },
        }
    }

    fn parse_cell(&mut self, _header: bool) -> ParsedCell {
        let (colspan, rowspan) = match self.tokens.get(self.position) {
            Some(Token::Start { attrs, .. }) => (
                span_attribute(attrs, "colspan"),
                span_attribute(attrs, "rowspan"),
            ),
            _ => (1, 1),
        };
        self.position += 1;

        let mut raw = String::new();
        let mut nested_table = None;

        while self.position < self.tokens.len() {
            match &self.tokens[self.position] {
                Token::End { name } if name == "td" || name == "th" => {
                    self.position += 1;
                    break;
                }
                Token::End { name } if name == "tr" || name == "table" => break,
                Token::Start { name, .. } if name == "table" => match self.parse_table() {
                    Ok(index) => {
                        if let Some(parent) = self.current_table
                            && let Some(table) = self.tables[index].as_mut()
                        {
                            table.nested_in = Some(format!("table-{}", parent + 1));
                        }
                        nested_table = Some(index);
                        raw.push_str(&format!("[table {}]", index + 1));
                    }
                    Err(_) => {
                        raw.push_str("[table]");
                    }
                },
                Token::Text(text) => {
                    raw.push_str(text);
                    self.position += 1;
                }
                Token::Start { name, .. } if name == "br" => {
                    raw.push('\u{0}');
                    self.position += 1;
                }
                _ => self.position += 1,
            }
        }

        ParsedCell {
            text: normalize_text(&raw),
            colspan,
            rowspan,
            nested_table,
        }
    }
}

fn span_attribute(attrs: &[(String, String)], name: &str) -> usize {
    let value = attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim())
        .unwrap_or("");

    match value.parse::<usize>() {
        Ok(0) | Err(_) => 1,
        Ok(value) => value.min(MAX_SPAN),
    }
}

fn normalize_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;

    for character in raw.chars() {
        if character == '\u{0}' {
            out.push('\n');
            pending_space = false;
        } else if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            pending_space = false;
            out.push(character);
        }
    }

    out.trim().to_owned()
}

fn tokenize(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut position = 0usize;

    while position < bytes.len() {
        if bytes[position] != b'<' {
            let start = position;
            while position < bytes.len() && bytes[position] != b'<' {
                position += 1;
            }
            tokens.push(Token::Text(decode_entities(&text[start..position])));
            continue;
        }

        if text[position..].starts_with("<!--") {
            position = find_after(text, position + 4, "-->");
            continue;
        }
        if text[position..].starts_with("<!") || text[position..].starts_with("<?") {
            position = find_after(text, position + 2, ">");
            continue;
        }

        if text[position..].starts_with("</") {
            let name_start = position + 2;
            let name_end = scan_name(bytes, name_start);
            let name = text[name_start..name_end].to_ascii_lowercase();
            position = find_after(text, name_end, ">");
            if !name.is_empty() {
                tokens.push(Token::End { name });
            }
            continue;
        }

        let name_start = position + 1;
        let name_end = scan_name(bytes, name_start);
        if name_end == name_start {
            // Not a tag; keep the angle bracket as text.
            tokens.push(Token::Text("<".to_owned()));
            position += 1;
            continue;
        }

        let name = text[name_start..name_end].to_ascii_lowercase();
        let (attrs, self_closing, next) = parse_attributes(text, name_end);
        tokens.push(Token::Start {
            name: name.clone(),
            attrs,
        });
        position = next;

        if !self_closing && (name == "script" || name == "style") {
            let closing = format!("</{name}");
            if let Some(offset) = find_case_insensitive(&text[position..], &closing) {
                position = find_after(text, position + offset, ">");
            } else {
                position = bytes.len();
            }
        }
    }

    tokens
}

fn scan_name(bytes: &[u8], mut position: usize) -> usize {
    while position < bytes.len() {
        let byte = bytes[position];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.') {
            position += 1;
        } else {
            break;
        }
    }
    position
}

fn parse_attributes(text: &str, mut position: usize) -> (Vec<(String, String)>, bool, usize) {
    let bytes = text.as_bytes();
    let mut attrs = Vec::new();
    let mut self_closing = false;

    loop {
        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if position >= bytes.len() {
            break;
        }
        if bytes[position] == b'>' {
            position += 1;
            break;
        }
        if bytes[position] == b'/' {
            self_closing = true;
            position += 1;
            continue;
        }

        let name_start = position;
        while position < bytes.len()
            && !bytes[position].is_ascii_whitespace()
            && !matches!(bytes[position], b'=' | b'>' | b'/')
        {
            position += 1;
        }
        if position == name_start {
            position += 1;
            continue;
        }
        let name = text[name_start..position].to_ascii_lowercase();

        while position < bytes.len() && bytes[position].is_ascii_whitespace() {
            position += 1;
        }

        let mut value = String::new();
        if position < bytes.len() && bytes[position] == b'=' {
            position += 1;
            while position < bytes.len() && bytes[position].is_ascii_whitespace() {
                position += 1;
            }
            if position < bytes.len() && (bytes[position] == b'"' || bytes[position] == b'\'') {
                let quote = bytes[position];
                position += 1;
                let value_start = position;
                while position < bytes.len() && bytes[position] != quote {
                    position += 1;
                }
                value = decode_entities(&text[value_start..position]);
                if position < bytes.len() {
                    position += 1;
                }
            } else {
                let value_start = position;
                while position < bytes.len()
                    && !bytes[position].is_ascii_whitespace()
                    && !matches!(bytes[position], b'>' | b'/')
                {
                    position += 1;
                }
                value = decode_entities(&text[value_start..position]);
            }
        }

        attrs.push((name, value));
    }

    (attrs, self_closing, position)
}

fn find_after(text: &str, start: usize, needle: &str) -> usize {
    match text[start..].find(needle) {
        Some(offset) => start + offset + needle.len(),
        None => text.len(),
    }
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

fn decode_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let Some(end) = after.find(';') else {
            out.push('&');
            rest = after;
            continue;
        };
        if end > 32 {
            out.push('&');
            rest = after;
            continue;
        }

        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => {
                entity[1..].parse::<u32>().ok().and_then(char::from_u32)
            }
            _ => None,
        };

        match decoded {
            Some(character) => {
                out.push(character);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }

    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::FormatId;

    fn source() -> SourceRef {
        SourceRef::new("page.html", FormatId::Html)
    }

    fn read_html(html: &str) -> TableSet {
        let options = ReadOptions {
            min_rows: 1,
            min_cols: 1,
            ..ReadOptions::default()
        };
        read(&source(), html.as_bytes(), &options).unwrap()
    }

    #[test]
    fn reads_a_simple_table_with_entities_and_whitespace() {
        let html = r#"
            <table>
              <caption> People &amp; Places </caption>
              <thead><tr><th>name</th><th>city</th></tr></thead>
              <tbody><tr><td> Alice </td><td>Z&#252;rich</td></tr></tbody>
            </table>
        "#;
        let set = read_html(html);
        assert_eq!(set.tables.len(), 1);
        let table = &set.tables[0];
        assert_eq!(table.column_names(), ["name", "city"]);
        assert_eq!(table.rows[0][0], CellValue::Text("Alice".into()));
        assert_eq!(table.rows[0][1], CellValue::Text("Zürich".into()));
        assert_eq!(table.source.caption.as_deref(), Some("People & Places"));
    }

    #[test]
    fn expands_colspan_and_rowspan() {
        let html = r#"
            <table>
              <tr><th>id</th><th colspan="2">name</th></tr>
              <tr><td>1</td><td rowspan="2">Alice</td><td>A</td></tr>
              <tr><td>2</td><td>B</td></tr>
            </table>
        "#;
        let set = read_html(html);
        let table = &set.tables[0];
        assert_eq!(table.column_names(), ["id", "name", "name_2"]);
        assert_eq!(table.rows[0][1], CellValue::Text("Alice".into()));
        assert_eq!(table.rows[1][1], CellValue::Text("Alice".into()));
        assert!(table.warnings.iter().any(|event| {
            event.code == "html.merged_cell_expanded" && event.message.contains("rowspan=2")
        }));
        assert!(
            !table.warnings.iter().any(|event| {
                event.code == "html.merged_cell_expanded" && event.message.contains("rowspan=1")
            }),
            "a header colspan must not also be reported as a merged-cell expansion"
        );
        assert!(table.warnings.iter().any(|event| {
            event.code == "html.header_colspan_expanded"
                && event.severity == crate::model::Severity::Info
        }));
        assert!(
            !table
                .warnings
                .iter()
                .any(|event| event.code == "html.duplicate_header"),
            "colspan expansion must not be reported as a duplicate header"
        );
    }

    #[test]
    fn genuine_duplicate_headers_are_still_warnings() {
        let html = r#"
            <table>
              <tr><th>name</th><th>name</th></tr>
              <tr><td>Alice</td><td>Bob</td></tr>
            </table>
        "#;
        let set = read_html(html);
        let table = &set.tables[0];
        assert_eq!(table.column_names(), ["name", "name_2"]);
        assert!(table.warnings.iter().any(|event| {
            event.code == "html.duplicate_header"
                && event.severity == crate::model::Severity::Warning
        }));
    }

    #[test]
    fn first_merge_policy_keeps_only_the_first_cell() {
        let html = r#"
            <table>
              <tr><th>id</th><th>name</th></tr>
              <tr><td>1</td><td rowspan="2">Alice</td></tr>
              <tr><td>2</td><td></td></tr>
            </table>
        "#;
        let options = ReadOptions {
            merge_policy: MergePolicy::First,
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &options).unwrap();
        let table = &set.tables[0];
        assert_eq!(table.rows[0][1], CellValue::Text("Alice".into()));
        assert_eq!(table.rows[1][1], CellValue::Null);
    }

    #[test]
    fn error_merge_policy_rejects_merged_cells() {
        let html = r#"
            <table>
              <tr><th>id</th><th>name</th></tr>
              <tr><td colspan="2">Alice</td></tr>
            </table>
        "#;
        let options = ReadOptions {
            merge_policy: MergePolicy::Error,
            ..ReadOptions::default()
        };
        let error = read(&source(), html.as_bytes(), &options).unwrap_err();
        assert!(error.to_string().contains("--merge-policy"));
    }

    #[test]
    fn nested_tables_are_extracted_in_document_order() {
        let html = r#"
            <table>
              <caption>Outer</caption>
              <tr><th>id</th><th>detail</th></tr>
              <tr><td>1</td><td><table><caption>Inner</caption><tr><th>x</th></tr><tr><td>y</td></tr></table></td></tr>
            </table>
        "#;
        let options = ReadOptions {
            sheets: SheetSelection::All,
            use_captions: true,
            min_rows: 1,
            min_cols: 1,
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &options).unwrap();
        assert_eq!(set.tables.len(), 2);
        assert_eq!(set.tables[0].name, "Outer");
        assert_eq!(set.tables[1].name, "Inner");
        assert_eq!(
            set.tables[0].rows[0][1],
            CellValue::Text("[table 2]".into())
        );
        assert_eq!(
            set.tables[1].source.nested_in.as_deref(),
            Some("table-1"),
            "the nested table should record its parent"
        );
        assert!(set.tables[0].warnings.iter().any(|event| {
            event.code == "html.nested_table_extracted"
                && event.severity == crate::model::Severity::Warning
        }));
    }

    #[test]
    fn layout_tables_are_skipped_and_reported() {
        let html = r#"
            <table><tr><td>logo</td><td>nav</td></tr></table>
            <table><caption>Data</caption><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>
        "#;
        let set = read(&source(), html.as_bytes(), &ReadOptions::default()).unwrap();
        assert_eq!(set.tables.len(), 1);
        assert_eq!(set.tables[0].name, "table-2");
        assert_eq!(set.skipped.len(), 1);
        assert_eq!(set.skipped[0].table, 1);
        assert_eq!(set.skipped[0].rows, 1);
        assert_eq!(set.skipped[0].columns, 2);
        assert_eq!(set.skipped[0].reason, "1 row(s) < --min-rows 2");
        assert!(
            set.tables[0]
                .warnings
                .iter()
                .any(|event| event.code == "html.table_skipped")
        );
    }

    #[test]
    fn header_row_can_be_selected_explicitly() {
        let html = r#"
            <table>
              <tr><td>Quarterly report</td><td></td></tr>
              <tr><th>id</th><th>amount</th></tr>
              <tr><td>1</td><td>10</td></tr>
            </table>
        "#;
        let options = ReadOptions {
            header_row: Some(2),
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &options).unwrap();
        assert_eq!(set.tables[0].column_names(), ["id", "amount"]);
        assert_eq!(set.tables[0].rows[0][0], CellValue::Text("1".into()));
    }

    #[test]
    fn no_header_generates_column_names() {
        let html = r#"
            <table>
              <tr><td>Alice</td><td>10</td></tr>
              <tr><td>Bob</td><td>20</td></tr>
            </table>
        "#;
        let options = ReadOptions {
            has_header: false,
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &options).unwrap();
        assert_eq!(set.tables[0].column_names(), ["column_1", "column_2"]);
        assert_eq!(set.tables[0].row_count(), 2);
    }

    #[test]
    fn table_selection_by_index_and_name() {
        let html = r#"
            <table><caption>One</caption><tr><th>a</th></tr><tr><td>1</td></tr></table>
            <table><caption>Two</caption><tr><th>b</th></tr><tr><td>2</td></tr></table>
        "#;
        let by_index = ReadOptions {
            sheets: SheetSelection::Index(2),
            use_captions: true,
            min_rows: 1,
            min_cols: 1,
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &by_index).unwrap();
        assert_eq!(set.tables[0].name, "Two");

        let by_name = ReadOptions {
            sheets: SheetSelection::Named("One".to_owned()),
            use_captions: true,
            min_rows: 1,
            min_cols: 1,
            ..ReadOptions::default()
        };
        let set = read(&source(), html.as_bytes(), &by_name).unwrap();
        assert_eq!(set.tables[0].rows[0][0], CellValue::Text("1".into()));
    }

    #[test]
    fn malformed_html_does_not_panic() {
        let html = "<table><tr><td>oops</table><table><tr><th>x</th></tr><tr><td>y</td></tr>";
        let set = read_html(html);
        assert!(!set.tables.is_empty());
    }

    #[test]
    fn entities_in_attributes_and_text_are_decoded() {
        let html = "<table><tr><th>a&amp;b</th></tr><tr><td colspan=\"1\">&#65;</td></tr></table>";
        let set = read_html(html);
        assert_eq!(set.tables[0].column_names(), ["a&b"]);
        assert_eq!(set.tables[0].rows[0][0], CellValue::Text("A".into()));
    }
}

//! Text and tables read from Office Open XML and OpenDocument files. Only the
//! document markup is parsed; macros and embedded objects are never loaded.
use super::{Content, TEXT_LIMIT};
use calamine::{DataRef, Reader, SheetType, SheetVisible};
use quick_xml::events::{BytesStart, Event};
use std::io::{Cursor, Read};

pub const SHEET_ROWS: usize = 100;
pub const SHEET_COLUMNS: usize = 20;
const SHEET_LIMIT: usize = 12;
const CELL_CHARS: usize = 80;
/// Cells read from one sheet before the rest is skipped.
const CELL_SCAN_LIMIT: usize = 1_000_000;
const PART_LIMIT: u64 = 32 * 1024 * 1024;
/// iced draws a tab character as a missing glyph.
const TAB: &str = "    ";
/// Declared size of a whole workbook, checked before the spreadsheet reader
/// loads its shared strings.
const WORKBOOK_LIMIT: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Paragraph(String),
}

#[derive(Debug, Clone, Default)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// The file's type label and content, or `None` when it is not an Office or
/// OpenDocument file Shep can read.
pub fn preview(bytes: &[u8]) -> Option<(&'static str, Content)> {
    if !bytes.starts_with(b"PK\x03\x04") {
        return None;
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    if archive.index_for_name("word/document.xml").is_some() {
        let xml = part(&mut archive, "word/document.xml")?;
        let mut collector = Collector::default();
        open_xml(&xml, &mut collector);
        return Some(("Word document", collector.finish()));
    }
    if archive.index_for_name("ppt/presentation.xml").is_some() {
        return Some(("PowerPoint presentation", slides(&mut archive)?));
    }
    if archive.index_for_name("xl/workbook.xml").is_some()
        || archive.index_for_name("xl/workbook.bin").is_some()
    {
        return Some(("Excel spreadsheet", workbook(&mut archive, bytes)?));
    }
    let mimetype = String::from_utf8(part(&mut archive, "mimetype")?).ok()?;
    let content = part(&mut archive, "content.xml")?;
    match mimetype.trim() {
        "application/vnd.oasis.opendocument.text" => {
            Some(("OpenDocument text", odf(&content, false)))
        }
        "application/vnd.oasis.opendocument.presentation" => {
            Some(("OpenDocument presentation", odf(&content, true)))
        }
        "application/vnd.oasis.opendocument.spreadsheet" => {
            Some(("OpenDocument spreadsheet", Content::Sheets(ods(&content))))
        }
        _ => None,
    }
}

fn part(archive: &mut Archive<'_>, name: &str) -> Option<Vec<u8>> {
    let file = archive.by_name(name).ok()?;
    if file.size() > PART_LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(PART_LIMIT).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Collects paragraphs up to the shared text preview limit.
#[derive(Default)]
struct Collector {
    blocks: Vec<Block>,
    current: String,
    heading: bool,
    used: usize,
    truncated: bool,
}
impl Collector {
    fn push(&mut self, text: &str) {
        if self.truncated {
            return;
        }
        let room = TEXT_LIMIT.saturating_sub(self.used);
        let text = if text.len() > room {
            self.truncated = true;
            let mut end = room;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            text
        };
        self.used += text.len();
        self.current.push_str(text);
    }

    fn paragraph(&mut self) {
        let text = std::mem::take(&mut self.current);
        let heading = std::mem::take(&mut self.heading);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.blocks.push(if heading {
            Block::Heading(text.to_owned())
        } else {
            Block::Paragraph(text.to_owned())
        });
    }

    fn section(&mut self, title: String) {
        self.paragraph();
        if !self.truncated {
            self.blocks.push(Block::Heading(title));
        }
    }

    fn finish(mut self) -> Content {
        self.paragraph();
        Content::Document {
            blocks: self.blocks,
            truncated: self.truncated,
        }
    }
}

/// The text of a character or entity reference such as `amp` or `#x41`.
fn reference(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let name = name.strip_prefix('#')?;
            let code = match name.strip_prefix('x').or_else(|| name.strip_prefix('X')) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => name.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// The text carried by an event, with entity references resolved.
fn text_of(event: &Event<'_>) -> Option<String> {
    match event {
        Event::Text(text) => Some(text.xml10_content().into_owned()),
        Event::CData(text) => Some(text.xml10_content().into_owned()),
        Event::GeneralRef(name) => reference(name).map(String::from),
        _ => None,
    }
}

/// Word body and PowerPoint slide markup share paragraph (`p`), run text
/// (`t`), tab and break element names.
fn open_xml(xml: &[u8], collector: &mut Collector) {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut in_text = false;
    // Alternate content repeats the same text box for older readers.
    let mut fallback = 0usize;
    loop {
        let event = match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(event) => event,
        };
        match &event {
            Event::Start(element) => match element.local_name().as_ref() {
                "Fallback" => fallback += 1,
                "t" if fallback == 0 => in_text = true,
                "p" if fallback == 0 => collector.paragraph(),
                _ => {}
            },
            Event::End(element) => match element.local_name().as_ref() {
                "Fallback" => fallback = fallback.saturating_sub(1),
                "t" => in_text = false,
                "p" if fallback == 0 => collector.paragraph(),
                _ => {}
            },
            Event::Empty(element) if fallback == 0 => match element.local_name().as_ref() {
                "tab" => collector.push(TAB),
                "br" | "cr" => collector.push("\n"),
                "pStyle" => {
                    collector.heading = attribute(element, "val").is_some_and(|style| {
                        let style = style.to_ascii_lowercase();
                        style.starts_with("heading") || style == "title" || style == "subtitle"
                    })
                }
                _ => {}
            },
            other if in_text && fallback == 0 => {
                if let Some(text) = text_of(other) {
                    collector.push(&text);
                }
            }
            _ => {}
        }
        buffer.clear();
    }
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|attribute| attribute.key.local_name().as_ref() == name)
        .and_then(|attribute| {
            attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
        })
        .map(|value| value.into_owned())
}

fn slides(archive: &mut Archive<'_>) -> Option<Content> {
    let mut slides: Vec<(usize, String)> = archive
        .file_names()
        .filter_map(|name| {
            let name = name.ok()?;
            let number = name
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse()
                .ok()?;
            Some((number, name.into_owned()))
        })
        .collect();
    slides.sort();
    let mut collector = Collector::default();
    for (position, (_, name)) in slides.iter().enumerate() {
        collector.section(format!("Slide {}", position + 1));
        if let Some(xml) = part(archive, name) {
            open_xml(&xml, &mut collector);
        }
    }
    Some(collector.finish())
}

/// OpenDocument text and presentation bodies.
fn odf(xml: &[u8], presentation: bool) -> Content {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut collector = Collector::default();
    let mut depth = 0usize;
    let mut skipped = 0usize;
    let mut slide = 0usize;
    loop {
        let event = match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(event) => event,
        };
        match &event {
            Event::Start(element) => match element.local_name().as_ref() {
                "annotation" | "notes" => skipped += 1,
                "page" if presentation => {
                    slide += 1;
                    collector.section(format!("Slide {slide}"));
                }
                name @ ("p" | "h") if skipped == 0 => {
                    collector.paragraph();
                    collector.heading = name == "h";
                    depth += 1;
                }
                _ => {}
            },
            Event::End(element) => match element.local_name().as_ref() {
                "annotation" | "notes" => skipped = skipped.saturating_sub(1),
                "p" | "h" if skipped == 0 => {
                    collector.paragraph();
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            },
            Event::Empty(element) if depth > 0 && skipped == 0 => {
                match element.local_name().as_ref() {
                    "s" => {
                        let count = attribute(element, "c")
                            .and_then(|count| count.parse::<usize>().ok())
                            .unwrap_or(1)
                            .min(64);
                        collector.push(&" ".repeat(count));
                    }
                    "tab" => collector.push(TAB),
                    "line-break" => collector.push("\n"),
                    _ => {}
                }
            }
            other if depth > 0 && skipped == 0 => {
                if let Some(text) = text_of(other) {
                    collector.push(&text);
                }
            }
            _ => {}
        }
        buffer.clear();
    }
    collector.finish()
}

/// Builds a bounded grid from cells that may arrive with gaps.
#[derive(Default)]
struct Grid {
    sheet: Sheet,
}
impl Grid {
    fn named(name: String) -> Self {
        Self {
            sheet: Sheet {
                name,
                ..Sheet::default()
            },
        }
    }

    fn set(&mut self, row: usize, column: usize, value: String) {
        if value.is_empty() {
            return;
        }
        if row >= SHEET_ROWS || column >= SHEET_COLUMNS {
            self.sheet.truncated = true;
            return;
        }
        let rows = &mut self.sheet.rows;
        if rows.len() <= row {
            rows.resize_with(row + 1, Vec::new);
        }
        let cells = &mut rows[row];
        if cells.len() <= column {
            cells.resize_with(column + 1, String::new);
        }
        cells[column] = value;
    }

    fn finish(mut self) -> Sheet {
        let width = self.sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
        for row in &mut self.sheet.rows {
            row.resize_with(width, String::new);
        }
        self.sheet
    }
}

fn cell(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(CELL_CHARS + 1)
        .collect();
    let value = value.trim();
    if value.chars().count() > CELL_CHARS {
        let mut cut: String = value.chars().take(CELL_CHARS - 1).collect();
        cut.push('…');
        cut
    } else {
        value.to_owned()
    }
}

fn data(value: &DataRef<'_>) -> String {
    match value {
        DataRef::Int(number) => number.to_string(),
        DataRef::Float(number) => number.to_string(),
        DataRef::String(text) | DataRef::DateTimeIso(text) | DataRef::DurationIso(text) => {
            cell(text)
        }
        DataRef::SharedString(text) => cell(text),
        DataRef::Bool(true) => "TRUE".into(),
        DataRef::Bool(false) => "FALSE".into(),
        DataRef::DateTime(date) => match date.as_datetime() {
            Some(moment) if moment.time() == chrono::NaiveTime::MIN => {
                moment.format("%Y-%m-%d").to_string()
            }
            Some(moment) => moment.format("%Y-%m-%d %H:%M").to_string(),
            None => date.to_string(),
        },
        DataRef::Error(error) => error.to_string(),
        DataRef::Empty => String::new(),
    }
}

fn workbook(archive: &mut Archive<'_>, bytes: &[u8]) -> Option<Content> {
    let declared: u64 = (0..archive.len())
        .filter_map(|index| archive.by_index_raw(index).ok().map(|file| file.size()))
        .sum();
    if declared > WORKBOOK_LIMIT {
        return Some(Content::Unsupported);
    }
    let visible = |sheets: &[calamine::Sheet]| -> Vec<String> {
        sheets
            .iter()
            .filter(|s| s.typ == SheetType::WorkSheet && s.visible == SheetVisible::Visible)
            .take(SHEET_LIMIT)
            .map(|s| s.name.clone())
            .collect()
    };
    // Cells are streamed: the dense range readers would allocate the whole
    // span between the first and last cell.
    let mut sheets = Vec::new();
    if archive.index_for_name("xl/workbook.bin").is_some() {
        let mut book: calamine::Xlsb<_> =
            calamine::open_workbook_from_rs(Cursor::new(bytes)).ok()?;
        for name in visible(book.sheets_metadata()) {
            let mut grid = Grid::named(name.clone());
            if let Ok(mut reader) = book.worksheet_cells_reader(&name) {
                for _ in 0..CELL_SCAN_LIMIT {
                    let Ok(Some(found)) = reader.next_cell() else {
                        break;
                    };
                    let (row, column) = found.get_position();
                    grid.set(row as usize, column as usize, data(found.get_value()));
                }
            }
            sheets.push(grid.finish());
        }
    } else {
        let mut book: calamine::Xlsx<_> =
            calamine::open_workbook_from_rs(Cursor::new(bytes)).ok()?;
        for name in visible(book.sheets_metadata()) {
            let mut grid = Grid::named(name.clone());
            if let Ok(mut reader) = book.worksheet_cells_reader(&name) {
                for _ in 0..CELL_SCAN_LIMIT {
                    let Ok(Some(found)) = reader.next_cell() else {
                        break;
                    };
                    let (row, column) = found.get_position();
                    grid.set(row as usize, column as usize, data(found.get_value()));
                }
            }
            sheets.push(grid.finish());
        }
    }
    Some(Content::Sheets(sheets))
}

/// OpenDocument spreadsheet tables. Repeated rows and cells are expanded only
/// up to the preview bounds, so trailing blank ranges cost nothing.
fn ods(xml: &[u8]) -> Vec<Sheet> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut sheets = Vec::new();
    let mut grid: Option<Grid> = None;
    let (mut row, mut column) = (0usize, 0usize);
    let mut row_repeat = 1usize;
    let mut cell_repeat = 1usize;
    let mut value: Option<String> = None;
    let mut paragraphs = 0usize;
    let repeat = |element: &BytesStart<'_>, name: &str| {
        attribute(element, name)
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or(1)
            .max(1)
    };
    loop {
        let event = match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) | Err(_) => break,
            Ok(event) => event,
        };
        match &event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                match element.local_name().as_ref() {
                    "table" if !empty => {
                        if sheets.len() >= SHEET_LIMIT {
                            break;
                        }
                        grid = Some(Grid::named(attribute(element, "name").unwrap_or_default()));
                        row = 0;
                    }
                    "table-row" => {
                        column = 0;
                        row_repeat = repeat(element, "number-rows-repeated");
                        if empty {
                            row = row.saturating_add(row_repeat);
                        }
                    }
                    "table-cell" | "covered-table-cell" => {
                        cell_repeat = repeat(element, "number-columns-repeated");
                        if empty {
                            column = column.saturating_add(cell_repeat);
                        } else {
                            value = Some(String::new());
                            paragraphs = 0;
                        }
                    }
                    "p" if value.is_some() && !empty => {
                        if paragraphs > 0
                            && let Some(value) = &mut value
                        {
                            value.push(' ');
                        }
                        paragraphs += 1;
                    }
                    "s" if empty => {
                        if let Some(value) = &mut value {
                            value.push(' ');
                        }
                    }
                    _ => {}
                }
            }
            Event::End(element) => match element.local_name().as_ref() {
                "table" => {
                    if let Some(grid) = grid.take() {
                        sheets.push(grid.finish());
                    }
                }
                "table-row" => {
                    // Repeated rows copy the row's cells; bounded by the grid.
                    if let Some(grid) = &mut grid {
                        let source = grid.sheet.rows.get(row).cloned();
                        for extra in 1..row_repeat.min(SHEET_ROWS) {
                            for (index, text) in source.iter().flatten().enumerate() {
                                grid.set(row + extra, index, text.clone());
                            }
                        }
                    }
                    row = row.saturating_add(row_repeat);
                }
                "table-cell" | "covered-table-cell" => {
                    let text = cell(&value.take().unwrap_or_default());
                    if let Some(grid) = &mut grid {
                        for extra in 0..cell_repeat.min(SHEET_COLUMNS) {
                            grid.set(row, column + extra, text.clone());
                        }
                    }
                    column = column.saturating_add(cell_repeat);
                }
                _ => {}
            },
            other => {
                if let (Some(value), Some(text)) = (&mut value, text_of(other))
                    && value.len() < CELL_CHARS * 4
                {
                    value.push_str(&text);
                }
            }
        }
        buffer.clear();
    }
    sheets
}

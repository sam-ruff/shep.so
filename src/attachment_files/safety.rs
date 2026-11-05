//! Decides whether an attachment may be handed to the default application.
//! Checks look at the content as well as the name, because a desktop can pick
//! an application from the content of a file without a familiar extension.
use super::{extension, pdf};
use quick_xml::events::Event;
use std::io::{Cursor, Read};

/// Why Shep keeps a file away from the default application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Program,
    ActiveContent,
    Unchecked,
}
impl Refusal {
    /// The error returned when opening is attempted anyway.
    pub fn error(self, name: &str) -> String {
        match self {
            Self::Program => format!(
                "Shep does not open programs or scripts such as {name}. Save a copy instead, and only run it if you trust the sender."
            ),
            Self::ActiveContent => format!(
                "{name} contains macros, scripts or embedded programs, so Shep does not open it. Save a copy only if you trust the sender."
            ),
            Self::Unchecked => format!(
                "Shep could not check {name} for macros or scripts, so it does not open it. Save a copy only if you trust the sender."
            ),
        }
    }

    /// A short explanation shown in the preview dialog.
    pub fn note(self) -> &'static str {
        match self {
            Self::Program => {
                "Programs and scripts can't be opened from Shep. Save a copy only if you trust the sender."
            }
            Self::ActiveContent => {
                "This file contains macros, scripts or embedded programs, so it can't be opened from Shep. Save a copy only if you trust the sender."
            }
            Self::Unchecked => {
                "Shep couldn't check this file for macros or scripts, so it can't be opened from Shep. Save a copy only if you trust the sender."
            }
        }
    }
}

/// Programs and scripts that the operating system could run instead of
/// display.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "exe",
    "com",
    "bat",
    "cmd",
    "msi",
    "msp",
    "mst",
    "scr",
    "pif",
    "cpl",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "mjs",
    "wsf",
    "wsh",
    "hta",
    "jar",
    "lnk",
    "url",
    "reg",
    "inf",
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "fish",
    "command",
    "tool",
    "desktop",
    "appimage",
    "run",
    "deb",
    "rpm",
    "apk",
    "app",
    "pkg",
    "mpkg",
    "py",
    "pyw",
    "pl",
    "rb",
    "php",
    "scpt",
    "applescript",
    "workflow",
    "gadget",
    "msc",
    "application",
    "appref-ms",
    "library-ms",
    "settingcontent-ms",
    "xbap",
    "dll",
    "so",
    "dylib",
    "elf",
    "html",
    "htm",
    "xhtml",
    "svg",
    "svgz",
    "mht",
    "mhtml",
];

/// Office formats whose purpose is to carry macros, plus add-ins.
const MACRO_EXTENSIONS: &[&str] = &[
    "docm", "dotm", "xlsm", "xltm", "xlam", "xla", "xll", "pptm", "potm", "ppsm", "ppam", "ppa",
    "sldm", "odb",
];

/// Extensions a user would expect to be a document Shep can check.
const OFFICE_EXTENSIONS: &[&str] = &[
    "doc", "docx", "dot", "dotx", "xls", "xlsx", "xlt", "xltx", "ppt", "pptx", "pps", "ppsx",
    "pot", "potx", "odt", "ott", "ods", "ots", "odp", "otp", "odg", "rtf", "pdf",
];

/// Zip parts that hold macros, ActiveX controls or embedded OLE programs.
const ACTIVE_PARTS: &[&str] = &[
    "vbaproject.bin",
    "vbadata.xml",
    "xl/macrosheets/",
    "activex/",
    "/embeddings/oleobject",
];

/// External relationship types that load code or another document on open.
const ACTIVE_RELATIONSHIPS: &[&str] =
    &["/attachedtemplate", "/oleobject", "/frame", "/subdocument"];

/// Compound-file storages and streams that hold macros or packaged programs.
const ACTIVE_STORAGES: &[&str] = &[
    "_vba_project",
    "_vba_project_cur",
    "vba",
    "macros",
    "\u{1}ole10native",
];

const PART_LIMIT: u64 = 8 * 1024 * 1024;

pub fn refusal(name: &str, bytes: &[u8]) -> Option<Refusal> {
    let extension = extension(name);
    let extension = extension.as_deref();
    if extension.is_some_and(|e| EXECUTABLE_EXTENSIONS.contains(&e)) || executable(bytes) {
        return Some(Refusal::Program);
    }
    if extension.is_some_and(|e| MACRO_EXTENSIONS.contains(&e)) {
        return Some(Refusal::ActiveContent);
    }
    let checked = if pdf::is_pdf(bytes) {
        pdf::active_content(bytes)
    } else if bytes.starts_with(b"PK\x03\x04") {
        zipped(bytes)
    } else if bytes.starts_with(&CFB) {
        compound(bytes)
    } else if bytes.starts_with(b"{\\rt") {
        rtf(bytes)
    } else if extension.is_some_and(|e| OFFICE_EXTENSIONS.contains(&e) && e != "pdf") {
        // A document name with content Shep does not recognise.
        Err(())
    } else {
        Ok(false)
    };
    match checked {
        Ok(false) => None,
        Ok(true) => Some(Refusal::ActiveContent),
        Err(()) => Some(Refusal::Unchecked),
    }
}

pub(super) const CFB: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

fn executable(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x7fELF")
        || bytes.starts_with(b"MZ")
        || bytes.starts_with(b"#!")
        || bytes.starts_with(&[0xcf, 0xfa, 0xed, 0xfe])
        || bytes.starts_with(&[0xca, 0xfe, 0xba, 0xbe])
}

/// `Ok(true)` when active content is present, `Err` when it can't be checked.
type Check = Result<bool, ()>;

fn zipped(bytes: &[u8]) -> Check {
    let Ok(mut archive) = zip::ZipArchive::new(Cursor::new(bytes)) else {
        return Err(());
    };
    let names: Vec<String> = archive
        .file_names()
        .map(|name| name.map(|name| name.to_ascii_lowercase()))
        .collect::<Result<_, _>>()
        .map_err(|_| ())?;
    let office = names
        .iter()
        .any(|n| n == "[content_types].xml" || n == "mimetype" || n == "meta-inf/manifest.xml");
    if !office {
        // An ordinary archive; the default app only lists its contents.
        return Ok(false);
    }
    if names.iter().any(|name| {
        ACTIVE_PARTS.iter().any(|part| name.contains(part))
            || name.starts_with("basic/")
            || name.starts_with("scripts/")
    }) {
        return Ok(true);
    }
    for index in 0..archive.len() {
        let Ok(mut file) = archive.by_index(index) else {
            return Err(());
        };
        let Ok(name) = file.name().map(|name| name.to_ascii_lowercase()) else {
            return Err(());
        };
        let relationships = name.ends_with(".rels");
        let content_types = name == "[content_types].xml";
        let odf = matches!(name.as_str(), "content.xml" | "styles.xml" | "settings.xml");
        if !(relationships || content_types || odf) {
            continue;
        }
        if file.size() > PART_LIMIT {
            return Err(());
        }
        let mut xml = Vec::new();
        if (&mut file).take(PART_LIMIT).read_to_end(&mut xml).is_err() {
            return Err(());
        }
        let lowered = String::from_utf8_lossy(&xml).to_ascii_lowercase();
        if content_types && lowered.contains("macroenabled") {
            return Ok(true);
        }
        if odf && (lowered.contains("script:event-listener") || lowered.contains("<office:script"))
        {
            return Ok(true);
        }
        if relationships && external_code(&xml)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether a relationships part loads a template, OLE object or frame from
/// outside the document, the route remote-template attacks use.
fn external_code(xml: &[u8]) -> Check {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element) | Event::Empty(element))
                if element.local_name().as_ref() == "Relationship" =>
            {
                let mut external = false;
                let mut active = false;
                for attribute in element.attributes().flatten() {
                    let value = attribute.value.to_ascii_lowercase();
                    match attribute.key.local_name().as_ref() {
                        "TargetMode" => external = value == "external",
                        "Type" => active = ACTIVE_RELATIONSHIPS.iter().any(|t| value.ends_with(t)),
                        _ => {}
                    }
                }
                if external && active {
                    return Ok(true);
                }
            }
            Ok(Event::Eof) => return Ok(false),
            Err(_) => return Err(()),
            Ok(_) => {}
        }
        buffer.clear();
    }
}

fn compound(bytes: &[u8]) -> Check {
    let Ok(mut file) = cfb::CompoundFile::open(Cursor::new(bytes)) else {
        return Err(());
    };
    let mut workbook = None;
    for entry in file.walk() {
        let name = entry.name().to_ascii_lowercase();
        if ACTIVE_STORAGES.contains(&name.as_str()) {
            return Ok(true);
        }
        // Legacy PowerPoint keeps macros inside its main stream.
        if name == "powerpoint document" {
            return Err(());
        }
        if entry.is_stream() && matches!(name.as_str(), "workbook" | "book") {
            workbook = Some(entry.path().to_path_buf());
        }
    }
    let Some(path) = workbook else {
        return Ok(false);
    };
    let mut stream = file.open_stream(path).map_err(|_| ())?;
    let mut data = Vec::new();
    stream
        .by_ref()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut data)
        .map_err(|_| ())?;
    excel4_macros(&data)
}

/// Looks for Excel 4.0 macro sheets and VBA modules among the sheets declared
/// in a BIFF workbook's global records.
fn excel4_macros(data: &[u8]) -> Check {
    const BOUNDSHEET: u16 = 0x0085;
    const EOF: u16 = 0x000a;
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let kind = u16::from_le_bytes([data[offset], data[offset + 1]]);
        let length = usize::from(u16::from_le_bytes([data[offset + 2], data[offset + 3]]));
        let body = data.get(offset + 4..offset + 4 + length).ok_or(())?;
        if kind == BOUNDSHEET {
            // Sheet type: 1 is a macro sheet, 6 a VBA module.
            if body.get(5).is_some_and(|sheet| matches!(sheet, 1 | 6)) {
                return Ok(true);
            }
        }
        if kind == EOF {
            return Ok(false);
        }
        offset += 4 + length;
    }
    Err(())
}

/// RTF cannot hold macros but can carry embedded OLE objects, a common
/// exploit route.
fn rtf(bytes: &[u8]) -> Check {
    let lowered = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    Ok(["\\object", "\\objdata", "\\objupdate", "\\objautlink"]
        .iter()
        .any(|word| lowered.contains(word)))
}

//! Attachment previews and private temporary copies opened with the system's
//! default application. Everything here runs in background workers.
pub mod office;
pub mod pdf;
mod safety;
#[cfg(test)]
mod tests;

pub use office::{Block, Sheet};
pub use safety::Refusal;

use anyhow::Context;
use iced::widget::image::Handle;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

/// Text previews stop here; the full file is available through Open or Save.
pub const TEXT_LIMIT: usize = 64 * 1024;
pub const TEXT_LINE_LIMIT: usize = 2_000;
/// Longest side of a prepared image, in pixels.
pub const IMAGE_SIDE: u32 = 2_048;
const DECODE_ALLOCATION: u64 = 256 * 1024 * 1024;
/// Larger attachments are only prepared when the user asks for them.
pub const PREPARE_LIMIT: usize = 16 * 1024 * 1024;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "text", "md", "markdown", "log", "csv", "tsv", "json", "xml", "yaml", "yml", "toml",
    "ini", "cfg", "conf", "ics", "vcf", "rs", "c", "h", "cpp", "hpp", "java", "kt", "go", "ts",
    "css", "sql", "diff", "patch", "srt", "tex", "rst", "eml",
];

#[derive(Clone)]
pub enum Content {
    Image {
        handle: Handle,
        width: u32,
        height: u32,
    },
    /// One rendered PDF page; other pages are prepared on request.
    Pages {
        handle: Handle,
        width: u32,
        height: u32,
        page: usize,
        pages: usize,
    },
    Text {
        text: String,
        truncated: bool,
    },
    Document {
        blocks: Vec<Block>,
        truncated: bool,
    },
    Sheets(Vec<Sheet>),
    Unsupported,
}
impl std::fmt::Debug for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image { width, height, .. } => write!(f, "Image({width}x{height})"),
            Self::Pages { page, pages, .. } => write!(f, "Page({} of {pages})", page + 1),
            Self::Text { text, truncated } => {
                write!(f, "Text({} bytes, truncated: {truncated})", text.len())
            }
            Self::Document { blocks, truncated } => {
                write!(
                    f,
                    "Document({} blocks, truncated: {truncated})",
                    blocks.len()
                )
            }
            Self::Sheets(sheets) => write!(f, "Sheets({})", sheets.len()),
            Self::Unsupported => f.write_str("Unsupported"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub name: String,
    pub size: usize,
    pub kind: &'static str,
    pub content: Content,
    pub refusal: Option<Refusal>,
}
impl Preview {
    pub fn openable(&self) -> bool {
        self.refusal.is_none()
    }

    /// Approximate memory held by the prepared content.
    pub fn weight(&self) -> usize {
        let content = match &self.content {
            Content::Image { width, height, .. } | Content::Pages { width, height, .. } => {
                *width as usize * *height as usize * 4
            }
            Content::Text { text, .. } => text.len(),
            Content::Document { blocks, .. } => blocks
                .iter()
                .map(|block| match block {
                    Block::Heading(text) | Block::Paragraph(text) => text.len(),
                })
                .sum(),
            Content::Sheets(sheets) => sheets
                .iter()
                .flat_map(|sheet| &sheet.rows)
                .flatten()
                .map(String::len)
                .sum(),
            Content::Unsupported => 0,
        };
        content + self.name.len()
    }
}

/// Builds the preview for one decoded attachment; `page` selects a PDF page.
/// Unreadable files fall back to the generic card instead of failing the
/// whole dialog.
pub fn preview(name: &str, bytes: &[u8], page: usize) -> Preview {
    let name = shep_mail_core::attachments::filename(name);
    let refusal = safety::refusal(&name, bytes);
    let (kind, content) = if let Some(content) = image(bytes) {
        ("Image", content)
    } else if pdf::is_pdf(bytes) {
        match pdf::page(bytes, page) {
            pdf::Rendered::Page(content) => ("PDF document", content),
            pdf::Rendered::Protected => ("Password-protected PDF", Content::Unsupported),
            pdf::Rendered::Unreadable => ("PDF document", Content::Unsupported),
        }
    } else if let Some(found) = office::preview(bytes) {
        found
    } else if is_text(&name, bytes) {
        ("Text", text(bytes))
    } else {
        (kind_label(&name), Content::Unsupported)
    };
    Preview {
        size: bytes.len(),
        name,
        kind,
        content,
        refusal,
    }
}

/// Whether the default application may receive this file.
pub fn openable(name: &str, bytes: &[u8]) -> bool {
    safety::refusal(name, bytes).is_none()
}

fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| extension.trim().to_ascii_lowercase())
}

fn kind_label(name: &str) -> &'static str {
    match extension(name).as_deref() {
        Some("pdf") => "PDF document",
        Some("doc" | "docx" | "odt" | "rtf") => "Document",
        Some("xls" | "xlsx" | "ods") => "Spreadsheet",
        Some("ppt" | "pptx" | "odp") => "Presentation",
        Some("zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "zst") => "Archive",
        Some("mp3" | "wav" | "ogg" | "flac" | "m4a" | "opus") => "Audio",
        Some("mp4" | "mkv" | "mov" | "webm" | "avi") => "Video",
        _ => "File",
    }
}

fn image(bytes: &[u8]) -> Option<Content> {
    let format = image::guess_format(bytes).ok()?;
    if !matches!(
        format,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP
    ) {
        return None;
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(DECODE_ALLOCATION);
    reader.limits(limits);
    let decoded = reader.decode().ok()?;
    let decoded = if decoded.width().max(decoded.height()) > IMAGE_SIDE {
        decoded.resize(
            IMAGE_SIDE,
            IMAGE_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        decoded
    };
    let rgba = decoded.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(Content::Image {
        handle: Handle::from_rgba(width, height, rgba.into_raw()),
        width,
        height,
    })
}

fn is_text(name: &str, bytes: &[u8]) -> bool {
    if let Some(extension) = extension(name) {
        return TEXT_EXTENSIONS.contains(&extension.as_str());
    }
    let sample = &bytes[..bytes.len().min(8 * 1024)];
    let sample = match std::str::from_utf8(sample) {
        Ok(sample) => sample,
        // A sample may end inside a multibyte character.
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&sample[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return false,
    };
    !sample.is_empty()
        && !sample
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
}

fn text(bytes: &[u8]) -> Content {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut end = bytes.len().min(TEXT_LIMIT);
    let mut truncated = end < bytes.len();
    if let Some((line_end, _)) = bytes[..end]
        .iter()
        .enumerate()
        .filter(|(_, b)| **b == b'\n')
        .nth(TEXT_LINE_LIMIT - 1)
    {
        end = line_end;
        truncated = true;
    }
    let mut text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    if truncated && text.ends_with('\u{fffd}') {
        text.pop();
    }
    Content::Text {
        text: expand_tabs(&text.replace("\r\n", "\n")),
        truncated,
    }
}

/// Replaces tabs with spaces up to the next eight-column stop, because iced
/// draws a tab character as a missing glyph.
fn expand_tabs(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_owned();
    }
    let mut expanded = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.chars() {
        match c {
            '\t' => {
                let spaces = 8 - column % 8;
                expanded.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            '\n' => {
                expanded.push(c);
                column = 0;
            }
            _ => {
                expanded.push(c);
                column += 1;
            }
        }
    }
    expanded
}

#[cfg_attr(test, mockall::automock)]
pub trait Launcher: Send + Sync {
    fn open(&self, path: &Path) -> anyhow::Result<()>;
}

pub struct SystemLauncher;
impl Launcher for SystemLauncher {
    fn open(&self, path: &Path) -> anyhow::Result<()> {
        open::that_detached(path).context(
            "Could not open the attachment with your default app. Save a copy and open it from your files instead.",
        )
    }
}

/// Private per-session copies, removed when the engine shuts down.
#[derive(Clone)]
pub struct Files {
    parent: Option<PathBuf>,
    root: Arc<OnceLock<Result<tempfile::TempDir, String>>>,
    launcher: Arc<dyn Launcher>,
}
impl Default for Files {
    fn default() -> Self {
        Self::new(None, Arc::new(SystemLauncher))
    }
}
impl Files {
    pub fn new(parent: Option<PathBuf>, launcher: Arc<dyn Launcher>) -> Self {
        Self {
            parent,
            root: Default::default(),
            launcher,
        }
    }

    /// Writes a private copy and, when `launch` is set, opens it with the
    /// default application. Returns the copy's path.
    pub fn open(&self, name: &str, bytes: &[u8], launch: bool) -> anyhow::Result<PathBuf> {
        let name = shep_mail_core::attachments::filename(name);
        if let Some(refusal) = safety::refusal(&name, bytes) {
            anyhow::bail!(refusal.error(&name));
        }
        let path = self.write(&name, bytes)?;
        if launch {
            self.launcher.open(&path)?;
        }
        Ok(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
        let root = self
            .root
            .get_or_init(|| {
                let mut builder = tempfile::Builder::new();
                builder.prefix("shep-attachments-");
                match &self.parent {
                    Some(parent) => builder.tempdir_in(parent),
                    None => builder.tempdir(),
                }
                .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| {
                anyhow::anyhow!(
                    "Could not create a temporary folder for attachments ({error}). Save a copy instead."
                )
            })?;
        // A separate folder per copy keeps the sender's file name intact.
        let folder = root.path().join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir(&folder)
            .context("Could not prepare a temporary copy. Save a copy instead.")?;
        let path = folder.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&path)
            .context("Could not write a temporary copy. Save a copy instead.")?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .context("Could not write a temporary copy. Save a copy instead.")?;
        Ok(path)
    }
}

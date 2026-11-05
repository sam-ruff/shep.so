//! PDF pages rendered with hayro, and checks for scripts and launch actions.
//! hayro only draws pages; it never runs a document's JavaScript.
use super::Content;
use hayro::hayro_syntax::{
    LoadPdfError, Pdf,
    object::{Dict, MaybeRef, Object},
};
use iced::widget::image::Handle;
use std::{panic::AssertUnwindSafe, sync::Arc};

/// Longest side of a rendered page, in pixels.
pub const PAGE_SIDE: f32 = 1_600.;
const NESTING_LIMIT: u8 = 32;

/// Names that run code, start programs or load interactive content.
const ACTIVE_NAMES: &[&[u8]] = &[b"JS", b"JavaScript", b"Launch", b"RichMedia", b"XFA"];

pub enum Rendered {
    Page(Content),
    Protected,
    Unreadable,
}

pub fn is_pdf(bytes: &[u8]) -> bool {
    // Readers accept a short preamble before the header.
    bytes[..bytes.len().min(1024)]
        .windows(5)
        .any(|window| window == b"%PDF-")
}

/// Renders one page; an out-of-range index shows the last page.
pub fn page(bytes: &[u8], index: usize) -> Rendered {
    // A malformed file must not take down the worker.
    std::panic::catch_unwind(AssertUnwindSafe(|| render(bytes, index)))
        .unwrap_or(Rendered::Unreadable)
}

fn render(bytes: &[u8], index: usize) -> Rendered {
    let pdf = match Pdf::new(Arc::new(bytes.to_vec())) {
        Ok(pdf) => pdf,
        Err(LoadPdfError::Decryption(_)) => return Rendered::Protected,
        Err(_) => return Rendered::Unreadable,
    };
    let pages = pdf.pages();
    let index = index.min(pages.len().saturating_sub(1));
    let Some(page) = pages.get(index) else {
        return Rendered::Unreadable;
    };
    let (width, height) = page.render_dimensions();
    if !(width.is_finite() && height.is_finite() && width >= 1. && height >= 1.) {
        return Rendered::Unreadable;
    }
    let scale = (PAGE_SIDE / width.max(height)).min(4.);
    let settings = hayro::RenderSettings {
        x_scale: scale,
        y_scale: scale,
        bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        ..Default::default()
    };
    let pixmap = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &hayro::hayro_interpret::InterpreterSettings::default(),
        &settings,
    );
    let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
    if width == 0 || height == 0 {
        return Rendered::Unreadable;
    }
    // The opaque white background makes premultiplied pixels plain RGBA.
    Rendered::Page(Content::Pages {
        handle: Handle::from_rgba(width, height, pixmap.data_as_u8_slice().to_vec()),
        width,
        height,
        page: index,
        pages: pages.len(),
    })
}

/// `Ok(true)` when the document carries scripts or launch actions, `Err` when
/// it can't be read well enough to tell.
pub(super) fn active_content(bytes: &[u8]) -> Result<bool, ()> {
    if raw_names(bytes) {
        return Ok(true);
    }
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let pdf = Pdf::new(Arc::new(bytes.to_vec())).map_err(|_| ())?;
        Ok(pdf.objects().into_iter().any(|object| active(&object, 0)))
    }))
    .unwrap_or(Err(()))
}

/// Finds active names written plainly in the file, including in objects the
/// parser might skip.
fn raw_names(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'/')
        .any(|(start, _)| {
            let rest = &bytes[start + 1..];
            ACTIVE_NAMES.iter().any(|name| {
                rest.starts_with(name)
                    && rest
                        .get(name.len())
                        .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'#')
            })
        })
}

fn active(object: &Object<'_>, depth: u8) -> bool {
    // Unreasonably deep nesting is treated as suspicious.
    if depth > NESTING_LIMIT {
        return true;
    }
    match object {
        Object::Name(name) => ACTIVE_NAMES.contains(&name.as_ref()),
        Object::Dict(dict) => dict_active(dict, depth),
        Object::Stream(stream) => dict_active(stream.dict(), depth),
        Object::Array(array) => array.raw_iter().any(|item| inline_active(&item, depth)),
        _ => false,
    }
}

fn dict_active(dict: &Dict<'_>, depth: u8) -> bool {
    dict.keys().any(|key| ACTIVE_NAMES.contains(&key.as_ref()))
        || dict
            .entries()
            .any(|(_, value)| inline_active(&value, depth))
}

/// References point at top-level objects, which are checked on their own.
fn inline_active(value: &MaybeRef<Object<'_>>, depth: u8) -> bool {
    match value {
        MaybeRef::NotRef(object) => active(object, depth + 1),
        MaybeRef::Ref(_) => false,
    }
}

//! The renderer has no network or filesystem resource loader. Images enter only
//! through the application's validated byte pipeline; CSS imports stay disabled.
use litehtml::*;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
pub(super) struct Surface(
    pub Rc<RefCell<shep_html_pixbuf::PixbufContainer>>,
    Rc<RefCell<String>>,
);
impl Surface {
    pub fn new(width: u32, height: u32, scale: f32) -> Self {
        Self(
            Rc::new(RefCell::new(
                shep_html_pixbuf::PixbufContainer::new_with_scale(width, height, scale),
            )),
            Rc::new(RefCell::new(String::new())),
        )
    }
}
impl Surface {
    fn resolve(&self, src: &str, base: &str) -> String {
        if let Some(cid) = src.strip_prefix("cid:") {
            return format!(
                "cid:{}",
                percent_encoding::percent_decode_str(cid).decode_utf8_lossy()
            );
        }
        if url::Url::parse(src).is_ok() {
            return src.into();
        }
        let base = if base.is_empty() {
            self.1.borrow().clone()
        } else {
            base.to_owned()
        };
        url::Url::parse(&base)
            .and_then(|u| u.join(src))
            .map(|u| u.to_string())
            .unwrap_or_else(|_| src.to_owned())
    }
}
macro_rules! delegate_mut {
    ($($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)+) => {$(
        fn $name(&mut self, $($arg: $ty),*) $(-> $ret)? { self.0.borrow_mut().$name($($arg),*) }
    )+};
}
impl DocumentContainer for Surface {
    delegate_mut! {
        create_font(descr: &FontDescription) -> (FontHandle, FontMetrics);
        delete_font(font: FontHandle);
        draw_text(hdc: DrawContext, text: &str, font: FontHandle, color: Color, pos: Position);
        draw_list_marker(hdc: DrawContext, marker: &ListMarker);
        draw_solid_fill(hdc: DrawContext, layer: &BackgroundLayer, color: Color);
        draw_linear_gradient(hdc: DrawContext, layer: &BackgroundLayer, gradient: &LinearGradient);
        draw_radial_gradient(hdc: DrawContext, layer: &BackgroundLayer, gradient: &RadialGradient);
        draw_conic_gradient(hdc: DrawContext, layer: &BackgroundLayer, gradient: &ConicGradient);
        draw_borders(hdc: DrawContext, borders: &Borders, draw_pos: Position, root: bool);
        set_cursor(cursor: &str);
        set_clip(pos: Position, radius: BorderRadiuses);
        del_clip();
    }
    fn set_base_url(&mut self, base: &str) {
        *self.1.borrow_mut() = base.to_owned();
    }
    fn on_anchor_click(&mut self, url: &str) {
        self.0.borrow_mut().on_anchor_click(&self.resolve(url, ""));
    }
    fn load_image(&mut self, src: &str, base: &str, redraw: bool) {
        let url = self.resolve(src, base);
        if url.starts_with("data:") && self.0.borrow().get_image_size(&url, "").width == 0. {
            use base64::Engine;
            if let Some((header, encoded)) = url.split_once(',')
                && matches!(
                    header,
                    "data:image/png;base64"
                        | "data:image/webp;base64"
                        | "data:image/jpeg;base64"
                        | "data:image/gif;base64"
                )
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
                && let Ok(webp) = crate::remote_images::convert_to_webp(&bytes)
            {
                self.0.borrow_mut().load_image_data(&url, &webp);
            }
        }
        self.0.borrow_mut().load_image(&url, "", redraw);
    }
    fn draw_image(&mut self, hdc: DrawContext, layer: &BackgroundLayer, url: &str, base: &str) {
        self.0
            .borrow_mut()
            .draw_image(hdc, layer, &self.resolve(url, base), "");
    }
    fn text_width(&self, text: &str, font: FontHandle) -> f32 {
        self.0.borrow().text_width(text, font)
    }
    fn get_image_size(&self, src: &str, baseurl: &str) -> Size {
        self.0
            .borrow()
            .get_image_size(&self.resolve(src, baseurl), "")
    }
    fn get_viewport(&self) -> Position {
        self.0.borrow().get_viewport()
    }
    fn get_media_features(&self) -> MediaFeatures {
        self.0.borrow().get_media_features()
    }
    fn default_font_name(&self) -> &str {
        "sans-serif"
    }
    fn transform_text(&self, text: &str, tt: TextTransform) -> String {
        self.0.borrow().transform_text(text, tt)
    }
}

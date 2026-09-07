//! Document-local caches. Misses always use the original shaping/rasterization
//! path; these limits affect retained work, never message content.
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Widths {
    fonts: HashMap<usize, HashMap<String, f32>>,
    count: usize,
    bytes: usize,
}
impl Widths {
    pub fn get(&self, font: usize, text: &str) -> Option<f32> {
        self.fonts.get(&font)?.get(text).copied()
    }
    pub fn insert(&mut self, font: usize, text: &str, width: f32) {
        if text.len() > 4096 || !width.is_finite() || self.get(font, text).is_some() {
            return;
        }
        if self.count >= 8192 || self.bytes + text.len() > 1024 * 1024 {
            *self = Self::default();
        }
        self.count += 1;
        self.bytes += text.len();
        self.fonts
            .entry(font)
            .or_default()
            .insert(text.into(), width);
    }
    pub fn remove_font(&mut self, font: usize) {
        if let Some(values) = self.fonts.remove(&font) {
            self.count -= values.len();
            self.bytes -= values.keys().map(String::len).sum::<usize>();
        }
    }
}

pub(crate) struct Glyphs {
    rasterizer: cosmic_text::SwashCache,
    images: HashMap<cosmic_text::CacheKey, Option<cosmic_text::SwashImage>>,
    bytes: usize,
}
impl Glyphs {
    pub fn new() -> Self {
        Self {
            rasterizer: cosmic_text::SwashCache::new(),
            images: HashMap::new(),
            bytes: 0,
        }
    }
    pub fn with_image<R>(
        &mut self,
        fonts: &mut cosmic_text::FontSystem,
        key: cosmic_text::CacheKey,
        use_image: impl FnOnce(&cosmic_text::SwashImage) -> R,
    ) -> Option<R> {
        const LIMIT: usize = 8 * 1024 * 1024;
        if !self.images.contains_key(&key) {
            let image = self.rasterizer.get_image_uncached(fonts, key);
            let bytes = image.as_ref().map_or(0, |image| image.data.len());
            if bytes > LIMIT {
                // Render an oversized glyph once, releasing it immediately.
                return image.as_ref().map(use_image);
            }
            if self.images.len() >= 2048 || self.bytes + bytes > LIMIT {
                self.images.clear();
                self.bytes = 0;
            }
            self.bytes += bytes;
            self.images.insert(key, image);
        }
        self.images
            .get(&key)
            .and_then(Option::as_ref)
            .map(use_image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widths_keep_font_and_unicode_identity_and_bound_retention() {
        let mut cache = Widths::default();
        cache.insert(1, "日本語", 24.);
        cache.insert(2, "日本語", 48.);
        assert_eq!(cache.get(1, "日本語"), Some(24.));
        assert_eq!(cache.get(2, "日本語"), Some(48.));
        cache.remove_font(1);
        assert_eq!(cache.get(1, "日本語"), None);
        assert_eq!(cache.get(2, "日本語"), Some(48.));
        for i in 0..20000 {
            cache.insert(i % 4, &format!("{i}{}", "α".repeat(100)), 12.);
            assert!(cache.count <= 8192 && cache.bytes <= 1024 * 1024);
        }
        let long = "x".repeat(5000);
        cache.insert(1, &long, 40000.);
        assert_eq!(cache.get(1, &long), None);
    }
    #[test]
    fn cached_glyphs_preserve_original_pixels_and_metrics() {
        use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};
        let mut fonts = cosmic_text::FontSystem::new();
        fonts
            .db_mut()
            .load_font_data(include_bytes!("../../../assets/NotoSans-Regular.ttf").to_vec());
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(18., 24.));
        buffer.set_text(
            &mut fonts,
            "Åffi 日本語",
            &Attrs::new().family(Family::Name("Noto Sans")),
            Shaping::Advanced,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let keys: Vec<_> = buffer
            .layout_runs()
            .flat_map(|run| {
                run.glyphs
                    .iter()
                    .map(|g| g.physical((0., 0.), 1.).cache_key)
            })
            .collect();
        assert!(!keys.is_empty());
        let mut original = cosmic_text::SwashCache::new();
        let mut cached = Glyphs::new();
        for _ in 0..3 {
            for &key in &keys {
                let expected = original.get_image_uncached(&mut fonts, key);
                let actual = cached.with_image(&mut fonts, key, |image| {
                    (
                        image.data.clone(),
                        image.placement.left,
                        image.placement.top,
                        image.placement.width,
                        image.placement.height,
                    )
                });
                assert_eq!(
                    actual,
                    expected.map(|image| (
                        image.data,
                        image.placement.left,
                        image.placement.top,
                        image.placement.width,
                        image.placement.height
                    ))
                );
            }
        }
        assert!(cached.bytes <= 8 * 1024 * 1024 && cached.images.len() <= keys.len());
    }
}

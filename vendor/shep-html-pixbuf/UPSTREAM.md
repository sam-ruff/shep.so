Adapted from `litehtml` 0.2.6 `src/pixbuf.rs` by Franz Geffke, MIT licensed.
Source: https://github.com/franzos/litehtml-rs

Shep owns this small drawing adapter because the upstream image painter crops
natural-size images and ignores CSS sizing, positioning, repetition and device
scale. The HTML/CSS engine remains the pinned upstream crate. Changes here honor
the computed image origin/clip boxes and scale the image pattern; document
convenience renderers are omitted. Production worker tests in `src/html_render/tests.rs`
cover this adapter through actual HTML, including scaled and inline images.
Preserve the license when copying or distributing this source.

Shep also exposes a worker-local shared FontSystem so opening another document
reuses font discovery without taking iced's UI font lock. Each document retains
independent font handles, glyphs and image resources. Same-size viewport repaints
clear and reuse the existing pixmap allocation instead of reallocating it.

Document-local text-width and glyph caches retain repeated layout/raster work.
Widths are scoped to font handles and evicted when a font is deleted; retention
is capped at 8,192 entries / 1 MiB of text. Glyph bitmaps retain at most 2,048
entries / 8 MiB. Oversized entries still render through the original path and
are released immediately. These are cache bounds, not message-content limits.
`cargo test -p shep-html-pixbuf` checks identity, eviction and unchanged glyph
pixels/metrics, and runs explicitly in hooks and quality CI.

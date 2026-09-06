Adapted from `litehtml` 0.2.6 `src/pixbuf.rs` by Franz Geffke, MIT licensed.
Source: https://github.com/franzos/litehtml-rs

Shep owns this small drawing adapter because the upstream image painter crops
natural-size images and ignores CSS sizing, positioning, repetition and device
scale. The HTML/CSS engine remains the pinned upstream crate. Changes here honor
the computed image origin/clip boxes and scale the image pattern; document
convenience renderers are omitted. Production worker tests in `src/html_render/tests.rs`
cover this adapter through actual HTML, including scaled and inline images.
Preserve the license when copying or distributing this source.

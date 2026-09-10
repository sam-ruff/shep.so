# Shep's iced software renderer patch

Source: crates.io `iced_tiny_skia` 0.14.0 from
https://github.com/iced-rs/iced/tree/0.14.0/tiny_skia (MIT; see LICENSE).
Cargo.toml and src/ are copied from that released crate. Cargo uses this copy
through the root `[patch.crates-io]`; the remaining iced crates stay pinned in
Cargo.lock. Do not edit the Cargo registry cache.

`src/engine.rs` fixes text and shadow clipping:

- Cached text always intersects its widget viewport with the current layer's
  damaged region, then applies that mask. A viewport does not describe the ink's
  bounds: dropdown labels can extend beyond it when scrolled off-screen.
- Raw text resets the shared mask to its own clip when needed. A preceding text
  item may have narrowed it.
- Shadows honor both damage and layer clipping, including damage that intersects
  only the shadow. Temporary shadow buffers cover only visible pixels.
  `src/layer.rs` includes shadows in damage bounds so moving or dismissing a
  control erases the entire old shadow without forcing a full repaint.

Keep `tests/software_rendering.rs`, the native filtered-preferences regression
and the scrolled mail-drag shadow regression
when updating iced. Remove this patch only after those tests pass with upstream.
These correctness tests do not replace the deferred idle-host performance gates.

`src/window/compositor.rs::group_damage` coalesces interleaved overlapping damage
against all pending regions, preserving the upstream extra-area budget and
separate distant changes. This avoids repainting a reader and each of its child
rectangles independently. Solid panels whose flat interior contains the damage
fill only those pixels; edges, shadows and gradients retain the general painter.
The existing clip mask preserves fractional-edge rounding. Direct renderer tests
compare full/partial repaint pixels and the solid/constant-gradient paths at
fractional scales. This keeps partial redraws; it never forces continuous full
window redraws. See docs/PERFORMANCE.md for native input-to-HTML evidence.

`src/vector.rs` keeps an unrotated SVG raster at physical scale, then applies the
complete position/rotation transform while compositing. Matrix diagonals include
cosine and cannot determine raster size or screen position. Raster cache keys
remain independent of animation angle. `src/engine.rs` tests rotated bounds for
visibility and intersects SVG viewport, layer and damage clips, restoring the
shared mask afterward. The refresh animation exposed the original misplaced
icons and trails. Keep the direct fractional-scale/partial-redraw SVG regressions
and native idle/background/manual animation pixel checks.

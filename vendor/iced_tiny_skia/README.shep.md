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

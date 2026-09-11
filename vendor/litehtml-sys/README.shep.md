# Shep litehtml layout patch

Vendored from crates.io `litehtml-sys` 0.2.5, used by the pinned `litehtml` 0.2.6 Rust wrapper. Upstream: https://github.com/franzos/litehtml-rs . The wrapper is MIT; bundled litehtml is BSD-3-Clause and Gumbo is Apache-2.0. Original licenses are retained here and included in release archives.

The local patch retains each table's last subtree layout for identical containing-block constraints within a single document layout phase. It compares every typed dimension, context index and sizing mode. It restores the table's own box after the generic render entry resets it; descendant positions remain relative. It never reuses a layout across document renders (including resize and newly loaded images) or the transition to positioned layout. This avoids exponential repeated work in deeply nested presentation tables without changing intrinsic or final width calculations.

Build tracking includes the bundled C/C++ sources. Do not edit the Cargo registry cache. Keep the paired uncached/cached layout, image reflow, resize, selection and native displayed-pixel regressions when updating this patch.

Caption displacement applies once to the cells, without accumulating another shift on their row parent. The saved top-caption height is refreshed even when zero, and a caption exactly as tall as its border still displaces the cells. Exact selection geometry, repeated-render and border-height regressions cover this correction. Both modes of the paired measurement tests include these offset corrections; they do not compare against pristine upstream output.

Inline fragment elements retain only their relative offset; `line_box.cpp` resets it before each application. This prevents wrapped fragments and repeated table measurements from accumulating a superscript/span offset. Keep the exact 5px/2px selection-geometry and repeated-layout regressions.

Windows MSVC builds follow upstream's CMake: Gumbo gets its `visualc/include` shim for the missing `<strings.h>`, and the C++ sources compile with `/utf-8 /permissive-`. The build script picks the C++ runtime from the target rather than the host: `c++` on macOS and iOS, none on MSVC, `stdc++` elsewhere.

# Logo assets

Generated with the built-in imagegen tool using the imagegen skill. The user approved the simplified light logo. Original generated PNGs are retained as sources; the app uses the 128 px WebP versions, decoded once at startup.

## Original prompt

Use case: logo-brand
Asset type: desktop email and calendar app logo
Primary request: a flat Swiss Shepherd side-profile dog head logo.
Subject: a White Swiss Shepherd (Berger Blanc Suisse) head in clear side profile, upright pointed ears, long elegant shepherd muzzle, calm alert expression, recognizable clean neck silhouette.
Style/medium: exceptionally clean flat vector-like logo rendered as a bitmap, minimal geometric shapes, tasteful negative space, strong silhouette legible at small icon sizes.
Scene/backdrop: genuinely transparent background.
Composition/framing: one centered head mark only, comfortably fills a square canvas, ample small even margin, no circle or badge.
Color palette: ivory white fur, deep charcoal outline and minimal facial details, a single subtle cool gray flat inner-ear plane.
Constraints: flat solid fills only, no gradients, no shadows, no texture, no realism, no lettering, no text, no watermark, no collar, no body. One logo only.

## Approved simplification prompt

Edit the provided Swiss Shepherd head logo. Preserve the White Swiss Shepherd subject and its right-facing side profile, pointed upright ears and overall recognizable head silhouette. Simplify it dramatically into an extremely minimal FLAT graphic logo: use only three uniform solid colors (ivory, deep charcoal, light gray), broad geometric shapes, no individual fur marks, no gradients whatsoever, no shading, no realistic eye, a tiny solid almond-shaped eye, no whisker dots, no highlights. At most 10 large closed shapes total. Make the neck a simple clean tapered shape. The result should resemble a refined modern app logo that reads clearly at 32 pixels. Center one head mark on a genuinely transparent square background, evenly padded. No text, no badges, no extra objects.

## Dark-mode prompt

Edit this approved flat Swiss Shepherd side-profile head logo to make its dark-mode counterpart. Preserve its exact geometry, proportions, right-facing pose, facial features and minimal flat design. Change only the palette and background: use pure solid deep charcoal #18181B as the entire background, light ivory #FAFAF9 for the outer silhouette/outline so it reads clearly on dark, a flat cool medium-gray #A1A1AA main head fill, flat charcoal inner ears, ivory simple eye and nose/mouth contrast. Keep the logo elegant and highly legible. Absolutely no checkerboard, no gradient, no texture, no shadow, no text. One centered logo at the same scale and framing. This is a dark-mode asset for a desktop application.

## Transparent desktop assets (R64)

The approved PNG references above are preserved. The original light PNG contains
an opaque checkerboard, and the original dark PNG contains an opaque background.
The built-in imagegen skill was used with these two background-extraction prompts:

- Light: remove only the exterior checkerboard, keep the exact dog geometry,
  scale, ivory/charcoal/gray palette and interior details, and output true alpha
  with smooth edges and no halo.
- Dark: remove only the exterior dark background, keep the approved outline,
  gray face, dark ear interiors and white details opaque, and output true alpha
  without changing the contour or framing.

The generated dark edge was visibly ragged and rejected. The user-authorized
[Vectorizer service](https://vectorizer.ai/api/documentation) traced the clean
light extraction and the approved dark reference. The dark background alone was
mapped to transparency; the light trace's interior opacity was restored to 100%.
The resulting `shepherd-light.svg` and `shepherd-dark.svg` preserve the recognizable
approved geometry with flat fills and clean contours. Original references remain
available for comparison; credentials stay in the original tooling configuration.

`shepherd-symbolic.svg` uses the same light contour and details with foreground
alpha levels for native system recoloring. GNOME/GTK chooses the foreground color,
including while Shep is hidden or closed. This follows the desktop's native icon
style; it does not write installed icons whenever the theme changes. The matching
32px WebP is a native macOS menu-bar template. Windows tray fallback and macOS/
Windows application launchers use the transparent full-color mark.

Run `uv run --with cairosvg --with pillow python scripts/build_icons.py` to export
128px lossless light/dark WebP, compatibility launcher PNGs and the 32px symbolic
WebP. This deterministic development script has no credential or network code;
the application decodes its small WebP once at startup. Preserve genuine alpha,
opaque full-color interiors and clean outer margins when rebuilding.

#!/usr/bin/env python3
"""Export reviewed Shepherd vector sources; development-only, no network calls.

uv run --with cairosvg --with pillow python scripts/build_icons.py
"""
from io import BytesIO
from pathlib import Path
import cairosvg
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]


def render(name, size):
    source = ROOT / 'assets' / f'shepherd-{name}.svg'
    # Supersampling preserves antialiased alpha for tiny desktop/tray contours.
    png = cairosvg.svg2png(url=str(source), output_width=size * 4, output_height=size * 4)
    return Image.open(BytesIO(png)).convert('RGBA').resize((size, size), Image.Resampling.LANCZOS)


ANDROID_RES = ROOT / 'flutter' / 'android' / 'app' / 'src' / 'main' / 'res'
ANDROID_DENSITIES = {'mdpi': 1, 'hdpi': 1.5, 'xhdpi': 2, 'xxhdpi': 3, 'xxxhdpi': 4}


def centred(name, canvas, artwork):
    image = Image.new('RGBA', (canvas, canvas), (0, 0, 0, 0))
    art = render(name, artwork)
    image.alpha_composite(art, ((canvas - artwork) // 2, (canvas - artwork) // 2))
    return image


def android_icons():
    for density, scale in ANDROID_DENSITIES.items():
        folder = ANDROID_RES / f'mipmap-{density}'
        # Adaptive layers are 108dp and launchers may mask outside the central 66dp;
        # the SVG's own padding keeps a 74dp render's artwork inside that circle.
        layer = round(108 * scale)
        centred('light', layer, round(74 * scale)).save(folder / 'ic_launcher_foreground.png')
        centred('symbolic', layer, round(74 * scale)).save(folder / 'ic_launcher_monochrome.png')
        legacy = round(48 * scale)
        tile = Image.new('RGBA', (legacy, legacy), (0, 0, 0, 0))
        ImageDraw.Draw(tile).rounded_rectangle(
            (0, 0, legacy - 1, legacy - 1), radius=legacy // 5, fill=(255, 255, 255, 255))
        tile.alpha_composite(centred('light', legacy, round(legacy * 0.8)))
        tile.save(folder / 'ic_launcher.png')


def main():
    assets = ROOT / 'assets'
    for name in ('light', 'dark'):
        icon = render(name, 128)
        icon.save(assets / f'logo-{name}.webp', format='WEBP', lossless=True, method=6)
        icon.save(assets / ('launcher.png' if name == 'light' else 'launcher-dark.png'))
    render('symbolic', 36).save(assets / 'logo-symbolic.webp', format='WEBP', lossless=True, method=6)
    for size in (16, 18, 20, 22, 24, 32, 36, 40, 44, 48, 64):
        render('tray', size).save(assets / f'tray-{size}.webp', format='WEBP', lossless=True, method=6)
    android_icons()


if __name__ == '__main__':
    main()

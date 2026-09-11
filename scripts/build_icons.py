#!/usr/bin/env python3
"""Export reviewed Shepherd vector sources; development-only, no network calls.

uv run --with cairosvg --with pillow python scripts/build_icons.py
"""
from io import BytesIO
from pathlib import Path
import cairosvg
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]


def render(name, size):
    source = ROOT / 'assets' / f'shepherd-{name}.svg'
    # Supersampling preserves antialiased alpha for tiny desktop/tray contours.
    png = cairosvg.svg2png(url=str(source), output_width=size * 4, output_height=size * 4)
    return Image.open(BytesIO(png)).convert('RGBA').resize((size, size), Image.Resampling.LANCZOS)


def main():
    assets = ROOT / 'assets'
    for name in ('light', 'dark'):
        icon = render(name, 128)
        icon.save(assets / f'logo-{name}.webp', format='WEBP', lossless=True, method=6)
        icon.save(assets / ('launcher.png' if name == 'light' else 'launcher-dark.png'))
    render('symbolic', 36).save(assets / 'logo-symbolic.webp', format='WEBP', lossless=True, method=6)
    for size in (16, 18, 20, 22, 24, 32, 36, 40, 44, 48, 64):
        render('tray', size).save(assets / f'tray-{size}.webp', format='WEBP', lossless=True, method=6)


if __name__ == '__main__':
    main()

"""Render the LiveChat icons (PNG and ICO) from the SVG sources in docs/brand/logo.

Usage (from the repository root, on Windows with Microsoft Edge and Pillow):

    python docs/brand/render_icons.py
    python docs/brand/render_icons.py --check

The SVGs are rasterised by headless Edge on a transparent page, one image per
target size (small sizes use the simplified logo, so they are drawn at their
real size rather than downscaled), then cropped and saved with Pillow.

`--check` writes nothing: it verifies that every source and every generated
icon is present, has the documented size, keeps a transparent background where
the charter expects one (tray and "sans fond" exports) and carries the tile
colour of its variant (Ardoise for the dark tile, #EEF1F7 for the light one).
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
LOGO = ROOT / "docs" / "brand" / "logo"
PNG = ROOT / "docs" / "brand" / "png"

APP = LOGO / "livechat-app-icon.svg"
APP_LIGHT = LOGO / "livechat-app-icon-light.svg"
APP_TRANSPARENT = LOGO / "livechat-app-icon-transparent.svg"
APP_SMALL = LOGO / "livechat-app-icon-small.svg"
SMALL_ON_DARK = LOGO / "livechat-logo-small-on-dark.svg"
SMALL_ON_LIGHT = LOGO / "livechat-logo-small-on-light.svg"

SOURCES = [APP, APP_LIGHT, APP_TRANSPARENT, APP_SMALL, SMALL_ON_DARK, SMALL_ON_LIGHT]

# (source svg, size in px) pairs to rasterise.
JOBS = sorted(
    {
        (APP_SMALL, 16),
        (APP_SMALL, 24),
        (APP_SMALL, 32),
        (APP, 48),
        (APP, 64),
        (APP, 128),
        (APP, 256),
        (APP, 512),
        (APP_LIGHT, 128),
        (APP_LIGHT, 512),
        (APP_TRANSPARENT, 128),
        (APP_TRANSPARENT, 512),
        (SMALL_ON_DARK, 32),
        (SMALL_ON_LIGHT, 32),
    },
    key=lambda job: (job[0].name, job[1]),
)

# The board is the overlay's main window; its icon also serves the installer.
ICO_SIZES = [(APP_SMALL, 16), (APP_SMALL, 24), (APP_SMALL, 32), (APP, 48), (APP, 64), (APP, 128), (APP, 256)]

OUTPUTS = {
    ROOT / "apps" / "lc-overlay" / "icons" / "icon.png": (APP, 512),
    # The tray icon has no tile: it follows the taskbar theme instead.
    ROOT / "apps" / "lc-overlay" / "icons" / "tray-dark.png": (SMALL_ON_DARK, 32),
    ROOT / "apps" / "lc-overlay" / "icons" / "tray-light.png": (SMALL_ON_LIGHT, 32),
    ROOT / "extension" / "public" / "icons" / "icon-16.png": (APP_SMALL, 16),
    ROOT / "extension" / "public" / "icons" / "icon-24.png": (APP_SMALL, 24),
    ROOT / "extension" / "public" / "icons" / "icon-32.png": (APP_SMALL, 32),
    ROOT / "extension" / "public" / "icons" / "icon-48.png": (APP, 48),
    ROOT / "extension" / "public" / "icons" / "icon-128.png": (APP, 128),
    # Exports for the store listing and the documentation, no tile.
    PNG / "app-icon-light-128.png": (APP_LIGHT, 128),
    PNG / "app-icon-light-512.png": (APP_LIGHT, 512),
    PNG / "app-icon-transparent-128.png": (APP_TRANSPARENT, 128),
    PNG / "app-icon-transparent-512.png": (APP_TRANSPARENT, 512),
}
ICO_OUTPUT = ROOT / "apps" / "lc-overlay" / "icons" / "icon.ico"

GAP = 8

TILE_DARK = (0x17, 0x1B, 0x25)
TILE_LIGHT = (0xEE, 0xF1, 0xF7)
AMBER = (0xF2, 0xB5, 0x44)
TOLERANCE = 6

# What each generated file must look like: (path, tile or None, size).
EXPECTATIONS = [
    (ROOT / "apps" / "lc-overlay" / "icons" / "icon.png", TILE_DARK, 512),
    (ROOT / "apps" / "lc-overlay" / "icons" / "tray-dark.png", None, 32),
    (ROOT / "apps" / "lc-overlay" / "icons" / "tray-light.png", None, 32),
    (ROOT / "extension" / "public" / "icons" / "icon-16.png", TILE_DARK, 16),
    (ROOT / "extension" / "public" / "icons" / "icon-24.png", TILE_DARK, 24),
    (ROOT / "extension" / "public" / "icons" / "icon-32.png", TILE_DARK, 32),
    (ROOT / "extension" / "public" / "icons" / "icon-48.png", TILE_DARK, 48),
    (ROOT / "extension" / "public" / "icons" / "icon-128.png", TILE_DARK, 128),
    (PNG / "app-icon-light-128.png", TILE_LIGHT, 128),
    (PNG / "app-icon-light-512.png", TILE_LIGHT, 512),
    (PNG / "app-icon-transparent-128.png", None, 128),
    (PNG / "app-icon-transparent-512.png", None, 512),
]


def find_edge() -> Path:
    for base in (os.environ.get("ProgramFiles(x86)"), os.environ.get("ProgramFiles")):
        if base:
            candidate = Path(base) / "Microsoft" / "Edge" / "Application" / "msedge.exe"
            if candidate.exists():
                return candidate
    sys.exit("Microsoft Edge not found")


def layout() -> tuple[list[tuple[Path, int, int, int]], int, int]:
    """Places every job on one row; returns (svg, size, x, y) and the page size."""
    placed = []
    x = GAP
    for svg, size in JOBS:
        placed.append((svg, size, x, GAP))
        x += size + GAP
    height = max(size for _, size in JOBS) + 2 * GAP
    return placed, x, height


def render() -> dict[tuple[Path, int], Image.Image]:
    placed, width, height = layout()
    work = Path(tempfile.mkdtemp(prefix="livechat-icons-"))
    try:
        imgs = "\n".join(
            f'<img src="{svg.as_uri()}" width="{size}" height="{size}" '
            f'style="position:absolute;left:{x}px;top:{y}px">'
            for svg, size, x, y in placed
        )
        page = work / "page.html"
        page.write_text(
            "<!DOCTYPE html><html><head><style>html,body{margin:0;background:transparent}"
            "</style></head><body>" + imgs + "</body></html>",
            encoding="utf-8",
        )
        shot = work / "shot.png"
        result = subprocess.run(
            [
                str(find_edge()),
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--allow-file-access-from-files",
                "--force-device-scale-factor=1",
                "--default-background-color=00000000",
                f"--window-size={width},{height}",
                f"--user-data-dir={work / 'profile'}",
                f"--screenshot={shot}",
                page.as_uri(),
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
        )
        if not shot.exists():
            sys.exit(f"Edge wrote no screenshot:\n{result.stdout}\n{result.stderr}")
        sheet = Image.open(shot).convert("RGBA")
        sheet.load()
        return {(svg, size): sheet.crop((x, y, x + size, y + size)) for svg, size, x, y in placed}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def near(pixel: tuple[int, int, int, int], colour: tuple[int, int, int]) -> bool:
    return all(abs(channel - expected) <= TOLERANCE for channel, expected in zip(pixel[:3], colour))


def check_icon(path: Path, tile: tuple[int, int, int] | None, size: int) -> list[str]:
    """Checks one generated icon; returns the problems found (empty when fine)."""
    if not path.exists():
        return [f"{path.relative_to(ROOT)} is missing"]
    with Image.open(path) as image:
        if image.size != (size, size):
            return [f"{path.relative_to(ROOT)} is {image.size[0]}x{image.size[1]}, expected {size}x{size}"]
        rgba = image.convert("RGBA")
        rgba.load()
        data = rgba.tobytes()

    total = size * size
    opaque = 0
    tile_pixels = 0
    amber = 0
    for index in range(0, len(data), 4):
        alpha = data[index + 3]
        if alpha > 8:
            opaque += 1
        pixel = data[index : index + 4]
        if tile is not None and alpha > 250 and near(pixel, tile):
            tile_pixels += 1
        if alpha > 250 and near(pixel, AMBER):
            amber += 1

    problems = []
    if amber == 0:
        problems.append(f"{path.relative_to(ROOT)} lost its amber bubble")
    if tile is None:
        if opaque >= total * 0.5:
            problems.append(f"{path.relative_to(ROOT)} should have no background but is {100 * opaque // total}% opaque")
    else:
        if tile_pixels < total * 0.15:
            problems.append(
                f"{path.relative_to(ROOT)} lost its #{tile[0]:02X}{tile[1]:02X}{tile[2]:02X} tile"
                f" ({100 * tile_pixels // total}% of pixels)"
            )
    return problems


def check() -> int:
    """Verifies the sources and the generated icons without writing anything."""
    problems: list[str] = []
    for svg in SOURCES:
        if not svg.exists():
            problems.append(f"{svg.relative_to(ROOT)} is missing")

    sources = {path: path.read_text(encoding="utf-8") for path in SOURCES if path.exists()}
    light = sources.get(APP_LIGHT, "")
    if light and f'fill="#EEF1F7"' not in light:
        problems.append(f"{APP_LIGHT.relative_to(ROOT)} must paint the light tile #EEF1F7")
    transparent = sources.get(APP_TRANSPARENT, "")
    if transparent and 'fill="#171B25"' in transparent:
        problems.append(f"{APP_TRANSPARENT.relative_to(ROOT)} must not paint a tile")
    if transparent and 'stroke="#171B25"' not in transparent:
        problems.append(f"{APP_TRANSPARENT.relative_to(ROOT)} must draw the corners in Ardoise")

    for path, tile, size in EXPECTATIONS:
        problems.extend(check_icon(path, tile, size))

    if not ICO_OUTPUT.exists():
        problems.append(f"{ICO_OUTPUT.relative_to(ROOT)} is missing")
    else:
        with Image.open(ICO_OUTPUT) as icon:
            frames = icon.info["sizes"]
        missing = [f"{size}x{size}" for _, size in ICO_SIZES if (size, size) not in frames]
        if missing:
            problems.append(f"{ICO_OUTPUT.relative_to(ROOT)} lacks the {', '.join(missing)} frames")

    if problems:
        for problem in problems:
            print(f"error: {problem}")
        return 1
    print(f"OK ({len(EXPECTATIONS)} icons, {len(ICO_SIZES)} ICO frames, {len(SOURCES)} sources)")
    return 0


def main() -> None:
    if "--check" in sys.argv[1:]:
        sys.exit(check())
    images = render()
    for path, job in OUTPUTS.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        images[job].save(path)
        print(f"wrote {path.relative_to(ROOT)}")
    frames = [images[job] for job in ICO_SIZES]
    largest = frames[-1]
    largest.save(
        ICO_OUTPUT,
        format="ICO",
        sizes=[frame.size for frame in frames],
        append_images=frames[:-1],
    )
    print(f"wrote {ICO_OUTPUT.relative_to(ROOT)} ({', '.join(str(f.width) for f in frames)})")


if __name__ == "__main__":
    main()

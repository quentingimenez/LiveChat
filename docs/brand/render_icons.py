"""Render the LiveChat icons (PNG and ICO) from the SVG sources in docs/brand/logo.

Usage (from the repository root, on Windows with Microsoft Edge and Pillow):

    python docs/brand/render_icons.py

The SVGs are rasterised by headless Edge on a transparent page, one image per
target size (small sizes use the simplified logo, so they are drawn at their
real size rather than downscaled), then cropped and saved with Pillow.
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

APP = LOGO / "livechat-app-icon.svg"
APP_SMALL = LOGO / "livechat-app-icon-small.svg"
SMALL_ON_DARK = LOGO / "livechat-logo-small-on-dark.svg"
SMALL_ON_LIGHT = LOGO / "livechat-logo-small-on-light.svg"

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
}
ICO_OUTPUT = ROOT / "apps" / "lc-overlay" / "icons" / "icon.ico"

GAP = 8


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


def main() -> None:
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

#!/usr/bin/env python3
import os
import sys
import zipfile
from pathlib import Path

TARGETS = ("chromium", "firefox")


def write_archive(dist_dir: Path, archive_path: Path) -> None:
    if archive_path.exists():
        archive_path.unlink()

    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in sorted(dist_dir.rglob("*")):
            if path.is_file():
                archive.write(path, arcname=path.relative_to(dist_dir))

    print(f"Archive créée: {archive_path}")


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    dist_root = repo_root / "dist"

    if not dist_root.is_dir():
        print("dist/ introuvable. Lance d'abord `npm run build`.", file=sys.stderr)
        return 1

    version = os.environ.get("npm_package_version", "0.0.0")
    release_dir = repo_root / "release"
    release_dir.mkdir(parents=True, exist_ok=True)

    for target in TARGETS:
        dist_dir = dist_root / target

        if not dist_dir.is_dir():
            print(f"dist/{target}/ introuvable. Lance d'abord `npm run build`.", file=sys.stderr)
            return 1

        write_archive(dist_dir, release_dir / f"livechat-extension-{target}-v{version}.zip")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

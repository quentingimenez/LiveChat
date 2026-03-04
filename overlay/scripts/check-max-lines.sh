#!/usr/bin/env bash
set -euo pipefail

max_lines="${1:-250}"
if [[ ! "$max_lines" =~ ^[0-9]+$ ]] || (( max_lines < 1 )); then
  echo "Usage: $0 [max_lines_positive_integer]"
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(
  find src renderer tests docs scripts -type f \
    \( -name '*.ts' -o -name '*.d.ts' -o -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.md' -o -name '*.sh' -o -name '*.json' \) \
    ! -path '*/dist/*' | sort
)

for root_file in \
  main.js \
  main.runtime.js \
  preload.js \
  protocol.js \
  package.json \
  eslint.config.cjs \
  .prettierrc.json \
  tsconfig.json \
  tsconfig.base.json \
  tsconfig.main.json \
  tsconfig.preload.json \
  tsconfig.renderer.json; do
  if [[ -f "$root_file" ]]; then
    files+=("$root_file")
  fi
done

echo "Checking file size limit: max ${max_lines} lines"

violations=0
for file in "${files[@]}"; do
  line_count=$(wc -l < "$file")
  if (( line_count > max_lines )); then
    printf '  - %s (%d lines)\n' "$file" "$line_count"
    violations=1
  fi
done

if (( violations )); then
  echo "Line-count check failed. Please split oversized files."
  exit 1
fi

echo "Line-count check passed (${#files[@]} files)."

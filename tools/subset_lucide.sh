#!/bin/sh
# Rebuilds assets/fonts/lucide-subset.ttf with just the icons listed in
# src/gui/theme.rs (mod icon). The full font also has glyphs for ordinary
# letters, which would shadow Inter, so only the icon codepoints are kept.
# Needs fonttools (pip install fonttools) and lucide-static's lucide.ttf:
#   tools/subset_lucide.sh path/to/lucide-static/font/lucide.ttf
set -e
cd "$(dirname "$0")/.."
cps=$(grep -o '\\u{e[0-9a-f]*}' src/gui/theme.rs | sed 's/\\u{\(.*\)}/U+\1/' | sort -u | paste -sd, -)
pyftsubset "$1" --unicodes="$cps" --output-file=assets/fonts/lucide-subset.ttf \
    --layout-features='' --no-hinting

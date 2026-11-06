#!/usr/bin/env bash
# Packs tesseract for the Windows exe: the MSYS2 build, every DLL it loads from MSYS2, the English
# model the Linux version reads with, and the licenses of all of it. Runs in the Windows
# workflow's MSYS2 UCRT64 shell, after `pacman -S mingw-w64-ucrt-x86_64-tesseract-ocr zip`.
#
#   bash .github/bundle-tesseract.sh OUT.zip
set -euo pipefail

out=$(realpath -m "$1")
prefix=/ucrt64
work=$(mktemp -d)
dir="$work/tesseract"
mkdir -p "$dir/tessdata" "$dir/licenses"

cp "$prefix/bin/tesseract.exe" "$dir/"
# ldd lists every DLL the exe loads, recursively. Windows' own DLLs are on every machine.
ldd "$prefix/bin/tesseract.exe" | awk -v p="$prefix/" 'index($3, p) == 1 { print $3 }' | sort -u |
  while read -r dll; do cp "$dll" "$dir/"; done

# The same model file as the Linux build's (tesseract-ocr/tessdata), checked byte for byte, so
# both versions read the reward screen alike.
curl -fsSL -o "$dir/tessdata/eng.traineddata" \
  https://raw.githubusercontent.com/tesseract-ocr/tessdata/4.1.0/eng.traineddata
echo "daa0c97d651c19fba3b25e81317cd697e9908c8208090c94c3905381c23fc047  $dir/tessdata/eng.traineddata" |
  sha256sum -c -
cp -r "$prefix/share/tessdata/configs" "$dir/tessdata/"

# Tesseract is Apache 2.0; its libraries carry their own licenses.
for f in "$dir"/*.exe "$dir"/*.dll; do pacman -Qqo "$prefix/bin/$(basename "$f")"; done | sort -u > "$work/packages"
while read -r pkg; do
  name=${pkg#mingw-w64-ucrt-x86_64-}
  if [ -d "$prefix/share/licenses/$name" ]; then cp -r "$prefix/share/licenses/$name" "$dir/licenses/"; fi
  pacman -Qi "$pkg" | grep -E '^(Name|Version|URL|Licenses)' >> "$dir/licenses/PACKAGES.txt"
  echo >> "$dir/licenses/PACKAGES.txt"
done < "$work/packages"

rm -f "$out"
(cd "$dir" && zip -qr9 "$out" .)
echo "$(ls "$dir" | wc -l) files, $(du -sh "$dir" | cut -f1) unpacked, $(du -h "$out" | cut -f1) zipped"
"$dir/tesseract.exe" --version | head -1

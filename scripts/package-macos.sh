#!/bin/bash
set -euo pipefail

app_path="${1:?Pass the built Movie Harbor.app path}"
output_path="${2:?Pass the DMG output path}"
test -d "$app_path/Contents/MacOS" || { echo "App bundle is missing" >&2; exit 1; }

# The linker signs only the executable. Seal the complete bundle before distribution.
codesign --force --deep --sign - "$app_path"
codesign --verify --deep --strict --verbose=2 "$app_path"

staging_dir="$(mktemp -d)"
mount_dir="$(mktemp -d)"
mounted=0
cleanup() {
  if [ "$mounted" -eq 1 ]; then hdiutil detach "$mount_dir" >/dev/null 2>&1 || true; fi
  rm -rf "$staging_dir" "$mount_dir"
}
trap cleanup EXIT

ditto "$app_path" "$staging_dir/Movie Harbor.app"
ln -s /Applications "$staging_dir/Applications"
mkdir -p "$(dirname "$output_path")"
hdiutil create -volname "Movie Harbor" -srcfolder "$staging_dir" -ov -format UDZO "$output_path" >/dev/null
hdiutil verify "$output_path" >/dev/null
hdiutil attach -readonly -nobrowse -mountpoint "$mount_dir" "$output_path" >/dev/null
mounted=1
codesign --verify --deep --strict --verbose=2 "$mount_dir/Movie Harbor.app"

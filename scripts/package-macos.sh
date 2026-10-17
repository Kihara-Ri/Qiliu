#!/usr/bin/env bash

set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
product_name="$(node -p "require('$project_dir/src-tauri/tauri.conf.json').productName")"
app_path="$project_dir/src-tauri/target/release/bundle/macos/$product_name.app"
version="$(node -p "require('$project_dir/package.json').version")"
output_dir="$project_dir/release"
output_path="$output_dir/$product_name $version.dmg"
package_dir="$(mktemp -d "${TMPDIR:-/tmp}/qiliu-dmg.XXXXXX")"

cleanup() {
  rm -rf "$package_dir"
}
trap cleanup EXIT

if [[ ! -d "$app_path" ]]; then
  echo "Missing app bundle: $app_path" >&2
  echo "Run: npm run tauri build -- --bundles app" >&2
  exit 1
fi

codesign --force --deep --sign - "$app_path"
codesign --verify --deep --strict --verbose=2 "$app_path"

mkdir -p "$output_dir"
cp -R "$app_path" "$package_dir/$product_name.app"
ln -s /Applications "$package_dir/Applications"

# hdiutil intermittently fails with "Resource busy" on CI runners (Spotlight or
# another process briefly holds the source folder); retry a few times.
for attempt in 1 2 3 4 5; do
  if hdiutil create \
    -volname "$product_name $version" \
    -srcfolder "$package_dir" \
    -ov \
    -format UDZO \
    "$output_path"; then
    break
  fi
  if [[ "$attempt" == 5 ]]; then
    echo "hdiutil create 连续失败 5 次" >&2
    exit 1
  fi
  echo "hdiutil create 失败（第 $attempt 次），稍后重试" >&2
  sleep $((attempt * 5))
done

shasum -a 256 "$output_path"

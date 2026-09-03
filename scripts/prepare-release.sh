#!/usr/bin/env bash

set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-}"

if [[ -z "$version" ]]; then
  echo "用法：npm run release:prepare -- 1.2.0" >&2
  exit 1
fi

cd "$project_dir"
node scripts/sync-version.mjs "$version"
npm run check
git diff --check

cat <<EOF

版本 $version 已同步并通过本地检查。
确认 CHANGELOG.md 后执行：

  git add .
  git commit -m "Release Qiliu $version"
  git tag "v$version"
  git push origin main "v$version"

标签推送后，Release 工作流会构建 macOS、Windows 和 Android 安装包。
EOF

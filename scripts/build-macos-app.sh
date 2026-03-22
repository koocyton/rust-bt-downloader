#!/usr/bin/env bash
# 在项目根目录执行：生成 dist/Rust Download.app
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

APP_NAME="Rust Download.app"
OUT_DIR="${OUT_DIR:-$ROOT/dist}"
APP_PATH="$OUT_DIR/$APP_NAME"
CONTENTS="$APP_PATH/Contents"

echo "==> cargo build --release"
cargo build --release

echo "==> 组装 $APP_NAME"
rm -rf "$APP_PATH"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources/assets"

cp "$ROOT/target/release/rust-download" "$CONTENTS/MacOS/"
chmod +x "$CONTENTS/MacOS/rust-download"

cp "$ROOT/packaging/macos/Info.plist" "$CONTENTS/Info.plist"

if [[ -f "$ROOT/packaging/macos/AppIcon.icns" ]]; then
  cp "$ROOT/packaging/macos/AppIcon.icns" "$CONTENTS/Resources/"
  echo "    已复制 AppIcon.icns"
fi

if [[ -d "$ROOT/assets" ]]; then
  cp -R "$ROOT/assets/"* "$CONTENTS/Resources/assets/" 2>/dev/null || true
  echo "    已复制 Resources/assets"
fi

if command -v codesign &>/dev/null; then
  echo "==> ad-hoc 签名（本机运行）"
  codesign --force --deep --sign - "$APP_PATH" || true
else
  echo "    （未找到 codesign，跳过签名）"
fi

echo ""
echo "完成: $APP_PATH"
echo "运行: open \"$APP_PATH\""

#!/bin/sh
# macOS(Apple Silicon) 앱 번들 생성: target/aarch64-apple-darwin/release/deid.app
#
#     sh assets/macos/make_app.sh
#
# 맨 실행파일은 아이콘이 없고, 번들 ID·권한 설명이 없어 macOS 가 데스크탑·문서·다운로드·
# 외장/네트워크 드라이브 접근을 막는다. 번들(Info.plist + 아이콘) + ad-hoc 서명으로 해결한다.
set -e
cd "$(dirname "$0")/../.."

TARGET=aarch64-apple-darwin
cargo build --release --target "$TARGET"

OUT="target/$TARGET/release"
APP="$OUT/deid.app"
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$OUT/deid" "$APP/Contents/MacOS/deid"
sed "s/__VERSION__/$VERSION/g" assets/macos/Info.plist > "$APP/Contents/Info.plist"

# 아이콘: app.ico 의 가장 큰 프레임(256px) → iconset → icns
TMP=$(mktemp -d)
ICONSET="$TMP/deid.iconset"
mkdir "$ICONSET"
sips -s format png assets/icon/app.ico --out "$TMP/base.png" >/dev/null
for s in 16 32 128 256; do
    sips -z $s $s "$TMP/base.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    [ $d -le 256 ] && sips -z $d $d "$TMP/base.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/deid.icns"
rm -rf "$TMP"

# ad-hoc 서명 — 권한 허용 기록(TCC)이 이 앱에 고정되도록
codesign --force --sign - "$APP"

echo "완료: $APP"

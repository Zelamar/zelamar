#!/usr/bin/env bash
# Puts the libwebrtc build that flutter_webrtc's Windows plugin downloads at
# configure time in place first, and only if its SHA-256 matches the pin
# below (ADR-0019). The plugin checks nothing, so without this a tampered
# release asset would end up inside Zelamar.
#
#   flutter pub get && bash tool/libwebrtc.sh
#
# Bumping flutter_webrtc changes the version it wants; this script then
# stops, and the new pin comes from GitHub's own digest of the asset:
#   gh api repos/webrtc-sdk/libwebrtc/releases/tags/<version> \
#     --jq '.assets[] | select(.name=="libwebrtc-win-x64-release.zip") | .digest'
set -euo pipefail

version=libwebrtc.m150.7871.03
asset=libwebrtc-win-x64-release.zip
sha256=4cd8fce2939b67b034200b124e4559cf343387a047118755925a0551328df84d

cd "$(dirname "$0")/.."
uri=$(grep -A1 '"name": "flutter_webrtc"' .dart_tool/package_config.json |
  sed -n 's/.*"rootUri": "file:\/\/\/\{0,1\}\(.*\)",*/\1/p')
if [ -z "$uri" ]; then
  echo "flutter_webrtc is not in .dart_tool/package_config.json: run flutter pub get" >&2
  exit 1
fi
plugin=$uri
if command -v cygpath >/dev/null; then
  plugin=$(cygpath -u "$uri")
fi
third_party="$plugin/third_party"

wanted=$(sed -n 's/^binary_version *= *//p' "$third_party/libwebrtc_version.ini" | tr -d '\r ')
if [ "$wanted" != "$version" ]; then
  echo "flutter_webrtc now wants $wanted, but $version is pinned here" >&2
  exit 1
fi

zip="$third_party/downloads/$asset"
mkdir -p "$third_party/downloads"
if [ ! -f "$zip" ]; then
  curl -fsSL --retry 3 -o "$zip.part" \
    "https://github.com/webrtc-sdk/libwebrtc/releases/download/$version/$asset"
  mv "$zip.part" "$zip"
fi
actual=$(sha256sum "$zip" | cut -d' ' -f1)
if [ "$actual" != "$sha256" ]; then
  rm -f "$zip"
  echo "libwebrtc: SHA-256 $actual does not match the pinned $sha256" >&2
  exit 1
fi
# Whatever was unpacked before is unpacked again from the checked archive.
rm -rf "$third_party/libwebrtc"
echo "libwebrtc $version verified ($sha256)"

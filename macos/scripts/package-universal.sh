#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
macos_dir="$(cd "$script_dir/.." && pwd)"
root_dir="$(cd "$macos_dir/.." && pwd)"

app_name="${APP_NAME:-TypeClaw}"
build_dir="${BUILD_DIR:-$macos_dir/build/release}"
cargo_target_dir="${CARGO_TARGET_DIR:-$root_dir/target}"
macos_deployment_target="${MACOS_DEPLOYMENT_TARGET:-13.0}"
codesign_identity="${CODESIGN_IDENTITY:--}"
archs="${TYPECLAW_MACOS_ARCHS:-arm64 x86_64}"
typeclaw_version="${TYPECLAW_VERSION:-$("$root_dir/scripts/typeclaw-version.sh")}"
typeclaw_bundle_version="${TYPECLAW_BUNDLE_VERSION:-$("$root_dir/scripts/typeclaw-bundle-version.sh")}"

app_bundle="$build_dir/$app_name.app"
app_executable="$app_bundle/Contents/MacOS/$app_name"
module_cache_dir="$build_dir/clang-module-cache"
dist_dir="$build_dir/dist"
zip_path="$dist_dir/$app_name-macos-universal.zip"

ffi_include_dir="$macos_dir/TypeClawFFI/include"
info_plist="$macos_dir/Resources/Info.plist"
info_plist_strings="$macos_dir/Resources/en.lproj/InfoPlist.strings"
pkginfo="$macos_dir/Resources/PkgInfo"
icon_source="$macos_dir/Resources/TypeClaw.png"

kit_sources=("$macos_dir"/Sources/TypeClawKit/*.swift)
agent_sources=("${kit_sources[@]}" "$macos_dir"/Sources/TypeClawAgent/*.swift)

rust_target_for_arch() {
    case "$1" in
        arm64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "unsupported macOS arch: $1" >&2; return 2 ;;
    esac
}

swift_target_for_arch() {
    echo "$1-apple-macosx$macos_deployment_target"
}

require_rust_target() {
    local target="$1"
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "missing Rust target '$target'; install it with: rustup target add $target" >&2
        return 2
    fi
}

build_rust_staticlib() {
    local rust_target="$1"
    CARGO_TARGET_DIR="$cargo_target_dir" cargo build --release -p typeclaw-ffi --target "$rust_target"
}

build_swift_executable() {
    local arch="$1"
    local rust_target="$2"
    local output="$3"
    shift 3

    local rust_staticlib="$cargo_target_dir/$rust_target/release/libtypeclaw_ffi.a"
    local swift_target
    swift_target="$(swift_target_for_arch "$arch")"

    mkdir -p "$(dirname "$output")" "$module_cache_dir/$arch"
    xcrun swiftc \
        -target "$swift_target" \
        -O -whole-module-optimization \
        -module-cache-path "$module_cache_dir/$arch" \
        -I "$ffi_include_dir" \
        "$@" \
        -Xlinker -force_load -Xlinker "$rust_staticlib" \
        -framework AppKit -framework ApplicationServices -framework Carbon -framework ServiceManagement -framework IOKit \
        -o "$output"
}

copy_bundle_resources() {
    rm -rf "$app_bundle"
    mkdir -p "$app_bundle/Contents/MacOS" "$app_bundle/Contents/Resources/en.lproj"
    cp "$info_plist" "$app_bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $typeclaw_version" "$app_bundle/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $typeclaw_bundle_version" "$app_bundle/Contents/Info.plist"
    if [[ "$typeclaw_bundle_version" == "1" ]]; then
        echo "error: refusing CFBundleVersion=1" >&2
        return 1
    fi
    cp "$info_plist_strings" "$app_bundle/Contents/Resources/en.lproj/InfoPlist.strings"
    cp "$pkginfo" "$app_bundle/Contents/PkgInfo"
    plutil -lint "$app_bundle/Contents/Info.plist"
}

build_icons() {
    local resources_dir="$app_bundle/Contents/Resources"
    local icon_tiff_dir="$build_dir/$app_name-icon-tiffs"
    local icon_multi_tiff="$build_dir/$app_name-icons.tiff"

    mkdir -p "$resources_dir" "$icon_tiff_dir"
    sips -s format tiff --resampleHeightWidth 16 16 "$icon_source" \
        --out "$icon_tiff_dir/icon_16.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 32 32 "$icon_source" \
        --out "$icon_tiff_dir/icon_32.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 48 48 "$icon_source" \
        --out "$icon_tiff_dir/icon_48.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 128 128 "$icon_source" \
        --out "$icon_tiff_dir/icon_128.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 256 256 "$icon_source" \
        --out "$icon_tiff_dir/icon_256.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 512 512 "$icon_source" \
        --out "$icon_tiff_dir/icon_512.tiff" >/dev/null
    sips -s format tiff --resampleHeightWidth 1024 1024 "$icon_source" \
        --out "$icon_tiff_dir/icon_1024.tiff" >/dev/null
    tiffutil -cat "$icon_tiff_dir"/icon_*.tiff -out "$icon_multi_tiff" >/dev/null 2>/dev/null
    tiff2icns "$icon_multi_tiff" "$resources_dir/TypeClaw.icns"
}

codesign_bundle() {
    if [[ "$codesign_identity" != "-" ]]; then
        echo "error: TypeClaw release packaging is ad-hoc only; set CODESIGN_IDENTITY=-" >&2
        return 2
    fi
    local codesign_args=(--force --sign "$codesign_identity")

    codesign "${codesign_args[@]}" "$app_bundle"
    codesign --verify --strict --verbose=2 "$app_bundle"
}

create_zip() {
    mkdir -p "$dist_dir"
    rm -f "$zip_path"
    ditto -c -k --keepParent "$app_bundle" "$zip_path"
}

copy_bundle_resources
build_icons

app_arch_outputs=()
for arch in $archs; do
    rust_target="$(rust_target_for_arch "$arch")"
    require_rust_target "$rust_target"
    build_rust_staticlib "$rust_target"

    arch_dir="$build_dir/$arch"
    arch_app_exec="$arch_dir/$app_name"
    build_swift_executable "$arch" "$rust_target" "$arch_app_exec" "${agent_sources[@]}"
    app_arch_outputs+=("$arch_app_exec")
done

xcrun lipo -create "${app_arch_outputs[@]}" -output "$app_executable"
chmod 755 "$app_executable"

for arch in $archs; do
    xcrun lipo "$app_executable" -verify_arch "$arch"
done

codesign_bundle
create_zip

file "$app_executable"
echo "app: $app_bundle"
echo "zip: $zip_path"

#!/usr/bin/env bash
# Linux 打包：在 Debian 12(bookworm) 容器内编译
#
# glibc 说明：Tauri 2 需要 webkit2gtk-4.1，该库在 Debian 12 才有（glibc 2.36）。
# 信创老系统（UOS 20 glibc 2.28 / 麒麟 V10 glibc 2.31）如需运行，建议用 CLI 版本。
#
# 用法: scripts/pack-linux.sh [arch]   arch ∈ x86_64(默认) | aarch64

set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="debian:12-slim"

echo "━━━ [linux-$ARCH] 前端构建 ━━━"
(cd frontend && pnpm install --silent && pnpm exec vite build)

echo "━━━ [linux-$ARCH] Debian 12 容器编译 ━━━"
mkdir -p .ci-cache/cargo target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo":/cargo \
    -e CARGO_HOME=/cargo \
    "$IMAGE" bash -exc '
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config file \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
        librsvg2-dev libssl-dev patchelf
    export PATH="/cargo/bin:$PATH"
    # 首次无缓存时安装 Rust（.ci-cache/cargo 被挂载为 CARGO_HOME）
    if ! command -v cargo >/dev/null 2>&1; then
        curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
    fi
    cargo --version
    cd /work/src-tauri
    cargo build --release
    strip target/release/xdoc-stress
'

BIN="src-tauri/target/release/xdoc-stress"
[ -f "$BIN" ] || { echo "❌ 编译产物不存在: $BIN"; exit 1; }

echo "━━━ [linux-$ARCH] glibc 版本 ━━━"
strings "$BIN" | grep -o "GLIBC_[0-9.]*" | sort -V | tail -1

echo "━━━ [linux-$ARCH] 打包 tar.xz ━━━"
VERSION=$(grep -m1 '^version' src-tauri/Cargo.toml | sed 's/version = "\(.*\)"/\1/')
OUT="dist/xdoc-stress_${VERSION}_linux-${ARCH}.tar.xz"
tar -cJf "$OUT" -C "$(dirname "$BIN")" "$(basename "$BIN")"
echo "✅ $OUT"

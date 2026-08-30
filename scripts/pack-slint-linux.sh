#!/usr/bin/env bash
# Slint GUI 版打包 —— Debian 10(glibc 2.28) 容器编译，兼容 UOS 20 / 麒麟 V10
# 纯 Rust + Slint 软件渲染，无 webkit/gtk 依赖
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="debian:10-slim"

echo "━━━ [slint-linux-$ARCH] Debian 10 (glibc 2.28) ━━━"
mkdir -p .ci-cache/cargo-slint target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo-slint":/cargo \
    -e CARGO_HOME=/cargo \
    -e RUSTUP_HOME=/cargo/rustup \
    "$IMAGE" bash -exc '
    # Debian 10 EOL:改指 archive 源
    printf "deb http://archive.debian.org/debian buster main\ndeb http://archive.debian.org/debian-security buster/updates main\n" > /etc/apt/sources.list
    n=0
    until apt-get -o Acquire::Check-Valid-Until=false update -qq; do
        n=$((n + 1)); [ "$n" -ge 3 ] && exit 1
        sleep 3
    done
    # Slint 只需 X11 基础库（软件渲染，无 OpenGL/GTK）
    apt-get -o Acquire::Retries=8 install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config file lld \
        libx11-dev libxext-dev libxft-dev libxrandr-dev libxcursor-dev libxi-dev \
        libfontconfig1-dev libfreetype6-dev
    export PATH="/cargo/bin:$PATH"
    if ! command -v cargo >/dev/null 2>&1; then
        curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
    fi
    cargo --version
    export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
    cargo build --release --manifest-path src-slint/Cargo.toml
    strip target/release/xdoc-stress
'

BIN="target/release/xdoc-stress"
[ -f "$BIN" ] || { echo "❌ 产物不存在: $BIN"; exit 1; }

echo "━━━ [slint-linux-$ARCH] glibc 断言 ━━━"
MAX=$(strings "$BIN" | grep -o "GLIBC_[0-9.]*" | sort -V | tail -1 | cut -d. -f3)
[ "$MAX" -le 28 ] && echo "✅ GLIBC_2.$MAX ≤ 2.28" || { echo "❌ GLIBC_2.$MAX > 2.28"; exit 1; }

VERSION="0.2.0"
OUT="dist/xdoc-stress_${VERSION}_linux-${ARCH}.tar.xz"
tar -cJf "$OUT" -C "$(dirname "$BIN")" "$(basename "$BIN")"
echo "✅ $OUT"

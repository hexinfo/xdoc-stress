#!/usr/bin/env bash
# CLI 版打包 —— Debian 10(glibc 2.28) 容器编译，兼容 UOS 20 / 麒麟 V10
# 无 GUI 依赖（不含 webkit/gtk），纯 Rust 二进制
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="debian:10-slim"

echo "━━━ [cli-linux-$ARCH] Debian 10 (glibc 2.28) ━━━"
mkdir -p .ci-cache/cargo-cli target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo-cli":/cargo \
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
    apt-get -o Acquire::Retries=8 install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config file
    export PATH="/cargo/bin:$PATH"
    if ! command -v cargo >/dev/null 2>&1; then
        curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
    fi
    cargo --version
    cargo build --release --manifest-path src-cli/Cargo.toml
    strip target/release/xdoc-stress
'

BIN="target/release/xdoc-stress"
[ -f "$BIN" ] || { echo "❌ 产物不存在: $BIN"; exit 1; }

echo "━━━ [cli-linux-$ARCH] glibc 断言 ━━━"
MAX=$(strings "$BIN" | grep -o "GLIBC_[0-9.]*" | sort -V | tail -1 | cut -d. -f3)
[ "$MAX" -le 28 ] && echo "✅ GLIBC_2.$MAX ≤ 2.28" || { echo "❌ GLIBC_2.$MAX > 2.28"; exit 1; }

VERSION="0.1.0"
OUT="dist/xdoc-stress-cli_${VERSION}_linux-${ARCH}.tar.xz"
tar -cJf "$OUT" -C "$(dirname "$BIN")" "$(basename "$BIN")"
echo "✅ $OUT"

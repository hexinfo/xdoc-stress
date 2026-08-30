#!/usr/bin/env bash
# Linux 打包（glibc 2.28 地板）：在 Debian 10 容器内编译，产出可跑在 glibc ≥ 2.28 的所有发行版
#
# 用法: scripts/pack-linux.sh [arch]   arch ∈ x86_64(默认) | aarch64
# 环境:
#   XDS_BUILD_IMAGE   构建镜像，默认 debian:10-slim（多架构）
#   XDS_DOCKER_PROXY  容器内代理

set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="${XDS_BUILD_IMAGE:-debian:10-slim}"
GLIBC_FLOOR=28

echo "━━━ [linux-$ARCH] 前端构建 ━━━"
(cd frontend && pnpm install --silent && pnpm exec vite build)

echo "━━━ [linux-$ARCH] Debian 10 容器编译 (glibc 2.28) ━━━"
mkdir -p .ci-cache/cargo target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo":/cargo \
    -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
    ${XDS_DOCKER_PROXY:+-e http_proxy=$XDS_DOCKER_PROXY -e https_proxy=$XDS_DOCKER_PROXY} \
    -e CARGO_HOME=/cargo \
    -e CARGO_TARGET_DIR=/work/target/linux-$ARCH \
    "$IMAGE" sh -exc '
    # Debian 10 EOL:改指 archive 源,重试 3 次
    printf "deb http://archive.debian.org/debian buster main\ndeb http://archive.debian.org/debian-security buster/updates main\n" > /etc/apt/sources.list
    n=0
    until apt-get -o Acquire::Check-Valid-Until=false update -qq; do
        n=$((n + 1)); [ "$n" -ge 3 ] && exit 1
        sleep 3
    done
    apt-get -o Acquire::Retries=8 install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
        libssl-dev patchelf file
    export PATH="/cargo/bin:$PATH"
    command -v cargo >/dev/null 2>&1 && cargo --version >/dev/null 2>&1 \
        || curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
    cd /work/src-tauri
    cargo build --release
    strip target/linux-'"$ARCH"'/release/xdoc-stress
    # 归属修正（容器内 root 写的文件要让宿主用户可读）
    chown -R "$HOST_UID:$HOST_GID" target/linux-'"$ARCH"'/release/xdoc-stress 2>/dev/null || true
'

BIN="src-tauri/target/linux-$ARCH/release/xdoc-stress"
[ -f "$BIN" ] || { echo "❌ 编译产物不存在: $BIN"; exit 1; }

echo "━━━ [linux-$ARCH] glibc 兼容性断言 (≤ 2.$GLIBC_FLOOR) ━━━"
MAX_GLIBC=$(strings "$BIN" | grep -o "GLIBC_[0-9.]*" | sort -V | tail -1 | cut -d. -f3)
if [ "$MAX_GLIBC" -gt "$GLIBC_FLOOR" ]; then
    echo "❌ GLIBC 需求 $MAX_GLIBC > 地板 2.$GLIBC_FLOOR，构建环境漂移！"
    exit 1
fi
echo "✅ GLIBC_2.$MAX_GLIBC ≤ 2.$GLIBC_FLOOR"

echo "━━━ [linux-$ARCH] 打包 tar.xz ━━━"
VERSION=$(grep -m1 '^version' src-tauri/Cargo.toml | sed 's/version = "\(.*\)"/\1/')
OUT="dist/xdoc-stress_${VERSION}_linux-${ARCH}.tar.xz"
tar -cJf "$OUT" -C "$(dirname "$BIN")" "$(basename "$BIN")"
echo "✅ $OUT"

#!/usr/bin/env bash
# Linux 打包：在 Debian 12(bookworm) 容器内编译
#
# glibc 说明：Tauri 2 需要 webkit2gtk-4.1，该库在 Debian 12 才有（glibc 2.36）。
# 信创老系统（UOS 20 glibc 2.28 / 麒麟 V10 glibc 2.31）如需运行，建议用 CLI 版本
#（xdoc-core-stress-test 项目编译的纯二进制，无 GUI 依赖）。
#
# 用法: scripts/pack-linux.sh [arch]   arch ∈ x86_64(默认) | aarch64
# 环境:
#   XDS_BUILD_IMAGE   构建镜像，默认 debian:12-slim（多架构）

set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="${XDS_BUILD_IMAGE:-debian:12-slim}"

echo "━━━ [linux-$ARCH] 前端构建 ━━━"
(cd frontend && pnpm install --silent && pnpm exec vite build)

echo "━━━ [linux-$ARCH] Debian 12 容器编译 ━━━"
mkdir -p .ci-cache/cargo target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo":/cargo \
    -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" \
    ${XDS_DOCKER_PROXY:+-e http_proxy=$XDS_DOCKER_PROXY -e https_proxy=$XDS_DOCKER_PROXY} \
    -e CARGO_HOME=/cargo \
    "$IMAGE" sh -exc '
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
        libssl-dev patchelf file sudo
    # 非 root 编译（uid 与宿主一致，产物归属正确）
    id builder >/dev/null 2>&1 || useradd -m -u "$HOST_UID" builder
    sudo -u "#$HOST_UID" env CARGO_HOME=/cargo PATH="/cargo/bin:$PATH" \
        cargo build --release --manifest-path /work/src-tauri/Cargo.toml
    strip /work/src-tauri/target/release/xdoc-stress
'

BIN="src-tauri/target/release/xdoc-stress"
[ -f "$BIN" ] || { echo "❌ 编译产物不存在: $BIN"; exit 1; }

echo "━━━ [linux-$ARCH] 打包 tar.xz ━━━"
VERSION=$(grep -m1 '^version' src-tauri/Cargo.toml | sed 's/version = "\(.*\)"/\1/')
OUT="dist/xdoc-stress_${VERSION}_linux-${ARCH}.tar.xz"
tar -cJf "$OUT" -C "$(dirname "$BIN")" "$(basename "$BIN")"
echo "✅ $OUT"

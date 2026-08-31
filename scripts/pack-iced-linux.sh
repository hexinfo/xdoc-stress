#!/usr/bin/env bash
# iced GUI 版打包 —— Debian 10(glibc 2.28) 容器编译，兼容 UOS 20 / 麒麟 V10
# 纯 Rust + iced tiny-skia 软件渲染（无 wgpu/GPU/GTK 依赖，仅 X11）
# 本地跑需容器能访问网络：若 docker 代理注入 127.0.0.1 不可达，可
#   docker run 前在容器内 export http_proxy=http://host.docker.internal:7890
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-x86_64}"
IMAGE="debian:10-slim"
# 本地跑若 archive.debian.org 被网络环境阻断,可导出 DEB10_MIRROR=mirrors.aliyun.com 走归档镜像
# (GitHub Actions 无需设置)
MIRROR_HOST="${DEB10_MIRROR:-archive.debian.org}"
if [ "$MIRROR_HOST" = "archive.debian.org" ]; then
    SOURCES="deb http://archive.debian.org/debian buster main\ndeb http://archive.debian.org/debian-security buster/updates main"
else
    SOURCES="deb http://$MIRROR_HOST/debian-archive/debian buster main\ndeb http://$MIRROR_HOST/debian-archive/debian-security buster/updates main"
fi

echo "━━━ [iced-linux-$ARCH] Debian 10 (glibc 2.28) ━━━"
mkdir -p .ci-cache/cargo-iced target dist

docker run --rm --platform "linux/$ARCH" \
    -v "$PWD":/work -w /work \
    -v "$PWD/.ci-cache/cargo-iced":/cargo \
    -e CARGO_HOME=/cargo \
    -e RUSTUP_HOME=/cargo/rustup \
    -e http_proxy= -e https_proxy= -e HTTP_PROXY= -e HTTPS_PROXY= \
    -e SOURCES_LIST="$SOURCES
" \
    -e DEB10_MIRROR="$MIRROR_HOST" \
    "$IMAGE" bash -exc '
    # Debian 10 EOL:改指 archive 源
    printf "$SOURCES_LIST" > /etc/apt/sources.list
    n=0
    until apt-get -o Acquire::Check-Valid-Until=false update -qq; do
        n=$((n + 1)); [ "$n" -ge 3 ] && exit 1
        sleep 3
    done
    # iced tiny-skia 软渲染:运行时仅用 X11(无 OpenGL/GTK);libwayland-dev 仅为
    # softbuffer 默认 feature(wayland-dlopen)编译期 pkg-config 探测所需
    apt-get -o Acquire::Retries=8 install -y -qq --no-install-recommends \
        build-essential curl ca-certificates xz-utils pkg-config file lld \
        libx11-dev libxext-dev libxft-dev libxrandr-dev libxcursor-dev libxi-dev \
        libfontconfig1-dev libfreetype6-dev libwayland-dev upx-ucl
    export PATH="/cargo/bin:$PATH"
    # 国内网络下 static.rust-lang.org 常被断流,镜像模式时工具链走 USTC 镜像
    if ! cargo --version >/dev/null 2>&1; then
        export RUSTUP_DIST_SERVER="${DEB10_RUSTUP_MIRROR:-https://mirrors.ustc.edu.cn/rust-static}"
        export RUSTUP_UPDATE_ROOT="${RUSTUP_DIST_SERVER}/rustup"
        if [ -n "$DEB10_MIRROR" ] && [ "$DEB10_MIRROR" != "archive.debian.org" ]; then
            # 国内模式:rustup-init 与工具链均走 USTC 镜像(sh.rustup.rs 常被断流)
            curl -sSf "${RUSTUP_UPDATE_ROOT}/dist/$(uname -m)-unknown-linux-gnu/rustup-init" -o /tmp/rustup-init
            chmod +x /tmp/rustup-init
            /tmp/rustup-init -y --profile minimal --default-toolchain stable --no-modify-path
        else
            curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
        fi
    fi
    if [ -n "$DEB10_MIRROR" ] && [ "$DEB10_MIRROR" != "archive.debian.org" ]; then
        # 国内模式:cargo crate 源切 rsproxy(无条件覆盖,防陈旧配置残留)
        cat > /cargo/config.toml <<'CFG'
[source.crates-io]
replace-with = "rsproxy"

[source.rsproxy]
registry = "sparse+https://rsproxy.cn/index/"
CFG
    fi
    cargo --version
    export RUSTFLAGS="-C link-arg=-fuse-ld=lld"
    cargo build --release --manifest-path src-iced/Cargo.toml
    strip target/release/xdoc-stress-iced
    # glibc 断言必须在 UPX 之前：压缩后字符串表不可读，strings 找不到 GLIBC 符号
    MAXVER=$(strings target/release/xdoc-stress-iced | grep -oE "GLIBC_[0-9]+(\.[0-9]+)*" | sort -uV | tail -1 || true)
    MAX=${MAXVER#GLIBC_}
    [ -n "$MAX" ] || { echo "❌ 未检测到 GLIBC 版本符号"; exit 1; }
    LE=$(printf "%s\n" "2.28" "$MAX" | sort -V | tail -1)
    [ "$LE" = "2.28" ] && echo "✅ GLIBC_$MAX ≤ 2.28" || { echo "❌ GLIBC_$MAX > 2.28"; exit 1; }
    # UPX 压缩（apt 安装）
    if command -v upx >/dev/null 2>&1; then
        upx --best --lzma target/release/xdoc-stress-iced 2>&1 | tail -1
    else
        echo "UPX not available, skip"
    fi
'

BIN="target/release/xdoc-stress-iced"
[ -f "$BIN" ] || { echo "❌ 产物不存在: $BIN"; exit 1; }

# 优先取 tag 名（Actions 里 GITHUB_REF_NAME=vX.Y.Z），本地构建回退 Cargo.toml 版本
VERSION="${GITHUB_REF_NAME#v}"
[ -n "$VERSION" ] || VERSION=$(/usr/bin/sed -n 's/^version = "\(.*\)"/\1/p' src-iced/Cargo.toml | head -1)
OUT="dist/xdoc-stress_${VERSION}_linux-${ARCH}.tar.xz"
# 归档内统一命名为 xdoc-stress(暂存目录重命名,兼容 macOS BSD tar 与 GNU tar)
STAGE="$(mktemp -d)"
cp "$BIN" "$STAGE/xdoc-stress"
tar -cJf "$OUT" -C "$STAGE" xdoc-stress
rm -rf "$STAGE"
echo "✅ $OUT"

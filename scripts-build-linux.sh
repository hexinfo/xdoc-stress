#!/bin/bash
# Linux 包构建（双架构 x86_64 + aarch64）
#   容器: rust:1-bookworm 官方多架构镜像（debian 12，glibc 2.36 ≥ 2.28 要求）
#   网络: --network host 复用宿主代理；apt 失败自动重试
#   权限: 非 root 编译（uid 与宿主一致，产物免 root）
set -e
cd "$(dirname "$0")"
UID_GID="$(id -u):$(id -g)"
DEPS="libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev pkg-config file"

build_arch() {
  local arch="$1" platform="$2" target="$3"
  echo "━━━ [$arch] 阶段1：系统依赖 ━━━"
  docker run --rm --network host --platform "$platform" \
    -v "$PWD":/home/builder/app -w /home/builder/app \
    rust:1-bookworm bash -lc "
      for i in 1 2 3; do apt-get update && break || sleep 3; done
      apt-get install -y --no-install-recommends $DEPS >/dev/null
      id builder >/dev/null 2>&1 || useradd -m -u $(id -u) builder
      mkdir -p src-tauri/target && chown -R $(id -u):$(id -g) src-tauri/target 2>/dev/null || true
    "

  echo "━━━ [$arch] 阶段2：cargo release（非 root） ━━━"
  docker run --rm --network host --platform "$platform" \
    -u "$UID_GID" -e HOME=/tmp -e CARGO_HOME=/tmp/.cargo \
    -v "$PWD":/home/builder/app -w /home/builder/app \
    rust:1-bookworm bash -lc "
      cargo build --release --manifest-path src-tauri/Cargo.toml --target $target &&
      strip src-tauri/target/$target/release/xdoc-stress &&
      echo '==> 产物:' && file src-tauri/target/$target/release/xdoc-stress
    "
}

build_arch x86_64 linux/amd64 x86_64-unknown-linux-gnu
build_arch aarch64 linux/arm64 aarch64-unknown-linux-gnu

echo "━━━ 全部产物 ━━━"
ls -la src-tauri/target/x86_64-unknown-linux-gnu/release/xdoc-stress src-tauri/target/aarch64-unknown-linux-gnu/release/xdoc-stress

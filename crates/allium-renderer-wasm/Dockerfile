# allium-renderer-wasm 浏览器构建。
# 构建上下文 = 仓库根（workspace），产物 = ES module 工厂 .js + .wasm。
#
# 构建链版本约束（勿随意升级，组合经过验证）：
# - emsdk 4.0.10 ↔ Rust 1.94 必须配对（binaryen 版本 vs LLVM 20 bulk-memory）。
# - EMCC_CFLAGS=-sSUPPORT_LONGJMP=wasm（skia 内嵌 libpng/freetype 的 longjmp ABI）。
# - bindgen 需 libclang-18（emsdk libc++ 头要求 Clang>=16）。
# - FORCE_SKIA_BUILD=1（0.97 wasm 预编译包 rename bug + 无 webp，故源码编译）。
#
# 注：crate 内 .cargo/config.toml 的链接 flag 只在「从 crate 目录调用 cargo」时
# 生效（cargo 自 CWD 向上查找 .cargo），故下方所有 cargo 调用都 cd 进 crate 目录。
FROM emscripten/emsdk:4.0.10

# 来自 emsdk 4.0.10 容器的基础环境
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    curl ca-certificates git python3 ninja-build cmake pkg-config xz-utils gnupg \
    && rm -rf /var/lib/apt/lists/*

# 从 apt.llvm.org 装 libclang-18 供 bindgen 用（jammy 默认源 Clang 最高 14）
RUN curl -fsSL https://apt.llvm.org/llvm-snapshot.gpg.key | gpg --dearmor -o /usr/share/keyrings/llvm.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/llvm.gpg] http://apt.llvm.org/jammy/ llvm-toolchain-jammy-18 main" > /etc/apt/sources.list.d/llvm.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends libclang-18-dev \
    && rm -rf /var/lib/apt/lists/*

# Rust 工具链
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y \
    --default-toolchain 1.94.0 --profile minimal -t wasm32-unknown-emscripten
ENV PATH=/root/.cargo/bin:$PATH

# FreeType 用 emcc 编译，静态库 + pkg-config，供 freetype-sys 链接。
# 版本固定 2.12.1 保证构建可复现。
RUN curl -sSfL https://download.savannah.gnu.org/releases/freetype/freetype-2.12.1.tar.xz -o /tmp/ft.tar.xz \
 && tar -xf /tmp/ft.tar.xz -C /tmp \
 && cd /tmp/freetype-2.12.1 \
 && emcmake cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DCMAKE_INSTALL_PREFIX=/opt/freetype -DBUILD_SHARED_LIBS=OFF \
      -DFT_DISABLE_ZLIB=ON -DFT_DISABLE_BZIP2=ON -DFT_DISABLE_PNG=ON \
      -DFT_DISABLE_HARFBUZZ=ON -DFT_DISABLE_BROTLI=ON \
 && cmake --build build && cmake --install build \
 && rm -rf /tmp/freetype-2.12.1 /tmp/ft.tar.xz

ENV PKG_CONFIG_PATH=/opt/freetype/lib/pkgconfig
ENV PKG_CONFIG_ALLOW_CROSS=1
ENV CLANG=/emsdk/upstream/bin/clang
ENV CLANGXX=/emsdk/upstream/bin/clang++
ENV LIBCLANG_PATH=/usr/lib/llvm-18/lib
ENV SKIA_USE_EMSCRIPTEN=1
ENV EMCC_CFLAGS="-sSUPPORT_LONGJMP=wasm"
ENV FORCE_SKIA_BUILD=1

# ---- 依赖缓存层 ----
# 只拷 workspace + 各 member 的 manifest + Cargo.lock + wasm crate 的 .cargo，
# 用占位源把 skia / freetype / serde 等重依赖编译固化进缓存层。
# 之后改 src/ 只重编各 crate 自身（秒级），不重编 skia（~15min）。
WORKDIR /build
COPY Cargo.toml Cargo.lock /build/
COPY crates/allium-renderer/Cargo.toml          /build/crates/allium-renderer/Cargo.toml
COPY crates/allium-renderer-host/Cargo.toml     /build/crates/allium-renderer-host/Cargo.toml
COPY crates/allium-renderer-cli/Cargo.toml      /build/crates/allium-renderer-cli/Cargo.toml
COPY crates/allium-renderer-wasm/Cargo.toml     /build/crates/allium-renderer-wasm/Cargo.toml
COPY crates/allium-renderer-wasm/.cargo         /build/crates/allium-renderer-wasm/.cargo

# 占位源：满足各 crate 的 lib/bin 目标，让 cargo 解析 workspace 并拉取依赖。
# allium-renderer 的 Cargo.toml 声明了多个 examples/tools 目标（均 required-features=dev，
# 默认不编），但 cargo 解析 manifest 时仍校验这些路径存在，故需一并占位。
RUN mkdir -p crates/allium-renderer/src crates/allium-renderer/examples \
             crates/allium-renderer/tools \
             crates/allium-renderer-host/src \
             crates/allium-renderer-cli/src crates/allium-renderer-wasm/src \
 && echo '' > crates/allium-renderer/src/lib.rs \
 && echo '' > crates/allium-renderer-host/src/lib.rs \
 && echo '' > crates/allium-renderer-wasm/src/lib.rs \
 && echo 'fn main() {}' > crates/allium-renderer-cli/src/main.rs \
 && echo 'fn main() {}' > crates/allium-renderer-wasm/src/main.rs \
 && for ex in bench_render compare_sdf render_bonds_honor; do \
      echo 'fn main() {}' > "crates/allium-renderer/examples/$ex.rs"; \
    done \
 && for t in calibrate_text check_fonts dump_outline_sdf render_deck \
             render_ranking_preview render_task043_layer render_profile; do \
      echo 'fn main() {}' > "crates/allium-renderer/tools/$t.rs"; \
    done

# 依赖层只编 --lib：编译并缓存 skia（~15min）+ 全部依赖，产出 rlib。
# rlib 无 emcc 最终链接，故不触发 .cargo/config.toml 的 --export=alr_* 检查
# （占位 lib 尚无这些符号）。最终 bin 链接留到真实源码层。
# 两层都从 crate 目录调用 → 同一份 .cargo rustflags → skia fingerprint 稳定、可复用。
# skia tarball 从 GitHub codeload 裸连偶断，依赖层用重试循环。
RUN cd crates/allium-renderer-wasm \
 && for i in 1 2 3 4 5 6 7 8; do \
      cargo build --release --target wasm32-unknown-emscripten \
        --no-default-features --features skia --lib && break; \
      echo "== build attempt $i failed, retrying =="; sleep 5; \
    done \
 && cargo build --release --target wasm32-unknown-emscripten \
      --no-default-features --features skia --lib

# ---- 真实源码层 ----
COPY crates/allium-renderer/src       /build/crates/allium-renderer/src
COPY crates/allium-renderer-host/src  /build/crates/allium-renderer-host/src
COPY crates/allium-renderer-wasm/src  /build/crates/allium-renderer-wasm/src

# 触碰源让 cargo 重编各 crate（COPY 不更新 mtime 的兜底）。
# 只编 --bin：bin 经 #[path] 内联 lib.rs，避免 rlib 边界使 #[no_mangle] 被内部化。
RUN find crates -name '*.rs' -exec touch {} + \
 && cd crates/allium-renderer-wasm \
 && cargo build --release --target wasm32-unknown-emscripten \
      --no-default-features --features skia --bin allium_renderer_wasm

# 产物归集到 /artifacts（build.sh 用 `docker create` + `docker cp` 取出）
RUN mkdir -p /artifacts \
 && cp target/wasm32-unknown-emscripten/release/allium_renderer_wasm.js  /artifacts/ \
 && cp target/wasm32-unknown-emscripten/release/allium_renderer_wasm.wasm /artifacts/ \
 && ls -la /artifacts

# Reproducible browser runtime build: Rust semantic core + minimal FreeType.
FROM emscripten/emsdk:4.0.10

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates curl git pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.94.0 --profile minimal \
        -t wasm32-unknown-emscripten
ENV PATH=/root/.cargo/bin:$PATH

WORKDIR /build
COPY . /build

RUN cd crates/allium-renderer-wasm \
    && bash build.sh --inside-container \
    && ls -la /artifacts

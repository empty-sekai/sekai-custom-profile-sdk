# Reproducible browser runtime build: Rust semantic core + minimal FreeType.
FROM emscripten/emsdk:4.0.10

RUN sed -i \
        -e 's|http://archive.ubuntu.com|https://archive.ubuntu.com|g' \
        -e 's|http://security.ubuntu.com|https://security.ubuntu.com|g' \
        /etc/apt/sources.list \
    && apt-get update -o Acquire::Retries=5 \
    && apt-get install -y --no-install-recommends \
        bash ca-certificates curl git pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain 1.94.0 --profile minimal \
        -t wasm32-unknown-emscripten
ENV PATH=/root/.cargo/bin:$PATH
ENV CARGO_HTTP_TIMEOUT=600 \
    CARGO_HTTP_LOW_SPEED_LIMIT=1 \
    CARGO_NET_RETRY=10

WORKDIR /build
COPY . /build

RUN cd crates/allium-renderer-wasm \
    && bash build.sh --inside-container \
    && ls -la /artifacts

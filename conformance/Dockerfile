# Build and run the buffa protobuf conformance tests.
#
# Usage:
#   docker build -t buffa-conformance \
#       --build-arg TOOLS_IMAGE=ghcr.io/anthropics/buffa/tools:v33.5 \
#       -f conformance/Dockerfile .
#   docker run --rm buffa-conformance

ARG TOOLS_IMAGE=ghcr.io/anthropics/buffa/tools:v33.5

# ── Stage 1: pull the pre-built tools image ───────────────────────────────
FROM ${TOOLS_IMAGE} AS tools

# ── Stage 2: build our conformance binary ─────────────────────────────────
FROM rust:1.87-slim AS rust-builder

# System deps for linking.
RUN apt-get update && apt-get install -y pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy protoc and proto files from the tools image.
COPY --from=tools /protoc /usr/local/bin/protoc
COPY --from=tools /protos /workspace/conformance/protos

# Copy the workspace source.
COPY Cargo.toml Cargo.lock ./
COPY buffa/             buffa/
COPY buffa-build/       buffa-build/
COPY buffa-codegen/     buffa-codegen/
COPY buffa-descriptor/  buffa-descriptor/
COPY buffa-test/        buffa-test/
COPY buffa-types/       buffa-types/
COPY protoc-gen-buffa/  protoc-gen-buffa/
COPY conformance/       conformance/

# All three builds share one target dir, so the dependencies and build scripts
# that do not depend on the features (buffa-codegen, buffa-build, syn, ...) are
# compiled once. Each binary is copied out before the next build overwrites it.
RUN mkdir /out \
    && cargo build --release --manifest-path conformance/Cargo.toml \
    && cp conformance/target/release/conformance /out/buffa-conformance \
    && cargo build --release --manifest-path conformance/Cargo.toml \
        --features table \
    && cp conformance/target/release/conformance /out/buffa-conformance-table \
    && cargo build --release --manifest-path conformance/Cargo.toml \
        --no-default-features \
    && cp conformance/target/release/conformance /out/buffa-conformance-nostd

# ── Stage 3: runtime image ────────────────────────────────────────────────
FROM ubuntu:24.04

RUN apt-get update && apt-get install -y libstdc++6 libgcc-s1 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=tools /conformance_test_runner      /usr/local/bin/conformance_test_runner
COPY --from=rust-builder /out/buffa-conformance        /usr/local/bin/buffa-conformance
COPY --from=rust-builder /out/buffa-conformance-nostd  /usr/local/bin/buffa-conformance-nostd
COPY --from=rust-builder /out/buffa-conformance-table  /usr/local/bin/buffa-conformance-table
COPY conformance/known_failures.txt             /known_failures.txt
COPY conformance/known_failures_nostd.txt       /known_failures_nostd.txt
COPY conformance/known_failures_table.txt       /known_failures_table.txt
COPY conformance/known_failures_view.txt        /known_failures_view.txt
COPY conformance/known_failures_lazy.txt        /known_failures_lazy.txt
COPY conformance/known_failures_view_json.txt   /known_failures_view_json.txt
COPY conformance/known_failures_text.txt        /known_failures_text.txt
COPY conformance/known_failures_reflect.txt      /known_failures_reflect.txt
COPY conformance/known_failures_view_vtable.txt  /known_failures_view_vtable.txt
COPY conformance/run-conformance.sh             /run-conformance.sh
RUN chmod +x /run-conformance.sh

ENTRYPOINT ["/run-conformance.sh"]

# Stress test: generate Rust code for the entire Google Cloud APIs proto set
# and compile it to validate correctness.
#
# This validates that buffa's codegen handles thousands of .proto files,
# deep package hierarchies, complex cross-package imports, and that the
# generated code actually compiles with rustc.
#
# Build:
#   docker build -t buffa-stress-googleapis -f stress/googleapis/Dockerfile .
#
# Run (output stays in container):
#   docker run --rm buffa-stress-googleapis
#
# Run with mounted output for inspection:
#   docker run --rm -v ./stress/googleapis/out:/out buffa-stress-googleapis

# ── Stage 1: build protoc-gen-buffa (static musl binary) ────────────────
FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y pkg-config musl-tools \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY buffa/             buffa/
COPY buffa-build/       buffa-build/
COPY buffa-codegen/     buffa-codegen/
COPY buffa-descriptor/  buffa-descriptor/
COPY buffa-test/        buffa-test/
COPY buffa-types/       buffa-types/
COPY protoc-gen-buffa/  protoc-gen-buffa/
COPY protoc-gen-buffa-packaging/ protoc-gen-buffa-packaging/

RUN cargo build --release --bin protoc-gen-buffa --target x86_64-unknown-linux-musl

# ── Stage 2: generate code with buf ─────────────────────────────────────
FROM bufbuild/buf:1.50.0 AS generator

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/protoc-gen-buffa /usr/local/bin/protoc-gen-buffa

WORKDIR /workspace
COPY stress/googleapis/buf.gen.yaml .

# Create gen dir so COPY in the next stage always succeeds.
RUN mkdir -p /workspace/gen

# Generate against the googleapis BSR module.
# NOTE: this RUN must succeed even when buf fails, so the report stage can
# still build and summarize the failure. report.sh reads the recorded
# "Generate exit code: N" line and exits non-zero on our behalf.
RUN set -eu; \
    if buf generate buf.build/googleapis/googleapis > /generate.log 2>&1; then \
        status=0; \
    else \
        status=$?; \
    fi; \
    echo "Generate exit code: ${status}" >> /generate.log; \
    cat /generate.log

# ── Stage 3: compile the generated code ─────────────────────────────────
FROM rust:1.85-slim AS compiler

WORKDIR /compile

# Copy buffa workspace source (needed as dependencies).
COPY Cargo.toml Cargo.lock ./
COPY buffa/             buffa/
COPY buffa-build/       buffa-build/
COPY buffa-codegen/     buffa-codegen/
COPY buffa-descriptor/  buffa-descriptor/
COPY buffa-test/        buffa-test/
COPY buffa-types/       buffa-types/
COPY protoc-gen-buffa/  protoc-gen-buffa/
COPY protoc-gen-buffa-packaging/ protoc-gen-buffa-packaging/

# Copy the generated code and module tree generator.
COPY --from=generator /workspace/gen gen/
COPY --from=generator /generate.log  generate.log
COPY stress/googleapis/gen_lib_rs.py gen_lib_rs.py
COPY stress/googleapis/exclude_from_compile.txt exclude_from_compile.txt
COPY stress/googleapis/Cargo.stress.toml Cargo.stress.toml

# Install tools needed for the compilation step.
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/protoc-gen-buffa /usr/local/bin/protoc-gen-buffa
RUN apt-get update && apt-get install -y python3 protobuf-compiler && rm -rf /var/lib/apt/lists/*
RUN mkdir -p src stress-crate && python3 gen_lib_rs.py gen/ "../gen/" > src/lib.rs
# Add the stress crate as a workspace member and compile.
# NOTE: like the generate step, this RUN records cargo's exit code but does
# not propagate it — report.sh is the single source of pass/fail.
RUN set -eu; \
    cp Cargo.stress.toml stress-crate/Cargo.toml; \
    mv src stress-crate/src; \
    mv gen stress-crate/gen; \
    sed -i 's|"protoc-gen-buffa",|"protoc-gen-buffa",\n    "stress-crate",|' Cargo.toml; \
    sed -i 's|path = "buffa"|path = "../buffa"|;s|path = "buffa-types"|path = "../buffa-types"|' stress-crate/Cargo.toml; \
    if cargo check -p googleapis-stress > /compile.log 2>&1; then \
        status=0; \
    else \
        status=$?; \
    fi; \
    echo "Compile exit code: ${status}" >> /compile.log; \
    cat /compile.log

# ── Stage 4: report results ────────────────────────────────────────────
FROM alpine:3.21

RUN apk add --no-cache coreutils

COPY --from=generator /generate.log /results/generate.log
COPY --from=compiler  /compile.log  /results/compile.log
COPY --from=compiler  /compile/stress-crate/src/lib.rs /results/lib.rs
COPY --from=generator /workspace/gen /results/gen
COPY stress/googleapis/report.sh /report.sh
RUN chmod +x /report.sh

ENTRYPOINT ["/report.sh"]

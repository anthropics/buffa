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
RUN buf generate buf.build/googleapis/googleapis 2>&1 | tee /generate.log; \
    echo "Exit code: $?" >> /generate.log

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
RUN cp Cargo.stress.toml stress-crate/Cargo.toml \
    && mv src stress-crate/src \
    && mv gen stress-crate/gen \
    && sed -i 's|"protoc-gen-buffa",|"protoc-gen-buffa",\n    "stress-crate",|' Cargo.toml \
    && sed -i 's|path = "buffa"|path = "../buffa"|;s|path = "buffa-types"|path = "../buffa-types"|' stress-crate/Cargo.toml \
    && { cargo check -p googleapis-stress 2>&1; echo "Compile exit code: $?"; } | tee /compile.log

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

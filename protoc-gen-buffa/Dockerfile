# Dockerfile for the protoc-gen-buffa remote plugin.
#
# Builds a minimal image suitable for pushing to the Buf Schema Registry
# as a custom remote plugin.
#
# Build (must target linux/amd64 for BSR compatibility):
#   docker build --platform linux/amd64 -t protoc-gen-buffa -f protoc-gen-buffa/Dockerfile .
#
# Push to BSR:
#   buf beta registry plugin push --visibility public --image protoc-gen-buffa
#
# Test locally:
#   echo '<CodeGeneratorRequest bytes>' | docker run -i protoc-gen-buffa

FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy workspace files needed for the build.
COPY Cargo.toml Cargo.lock ./
COPY buffa/           buffa/
COPY buffa-build/     buffa-build/
COPY buffa-codegen/   buffa-codegen/
COPY buffa-descriptor/ buffa-descriptor/
COPY buffa-test/      buffa-test/
COPY buffa-types/     buffa-types/
COPY protoc-gen-buffa/ protoc-gen-buffa/
COPY protoc-gen-buffa-packaging/ protoc-gen-buffa-packaging/

RUN cargo build --release --bin protoc-gen-buffa

# Minimal runtime image — no OS, no shell, just the binary.
FROM scratch
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /build/target/release/protoc-gen-buffa /protoc-gen-buffa
USER nobody
ENTRYPOINT ["/protoc-gen-buffa"]

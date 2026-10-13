# Test protoc-gen-buffa against multiple protoc versions.
#
# Downloads pre-built protoc binaries from the protobuf GitHub releases,
# builds protoc-gen-buffa, then runs each protoc version against test
# proto files to verify the plugin protocol is compatible.
#
# Usage:
#   docker build -t buffa-protoc-compat -f compat/Dockerfile .
#   docker run --rm buffa-protoc-compat

# ── Stage 1: Download protoc binaries ──────────────────────────────────────

FROM debian:bookworm-slim AS fetcher

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl unzip ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Detect architecture for download URL.
ARG TARGETARCH

# Versions to test, from oldest supported to latest.
#   v21.12 — Debian Bookworm / Ubuntu 24.04 distro package
#   v22.5  — editions feature flag introduced (but editions experimental)
#   v25.5  — earliest editions 2023 support (experimental)
#   v27.3  — editions stable
#   v29.5  — matches common Nix/distro installs
#   v33.5  — latest, matches our tools image
ARG PROTOC_VERSIONS="21.12 22.5 25.5 27.3 29.5 33.5"

# Download each protoc version into /protoc/<version>/bin/protoc.
RUN set -e; \
    ARCH=$(case "$TARGETARCH" in \
      amd64) echo "x86_64" ;; \
      arm64) echo "aarch_64" ;; \
      *) echo "unsupported arch: $TARGETARCH" >&2; exit 1 ;; \
    esac); \
    for ver in $PROTOC_VERSIONS; do \
      echo "==> Downloading protoc v${ver} (linux-${ARCH})"; \
      url="https://github.com/protocolbuffers/protobuf/releases/download/v${ver}/protoc-${ver}-linux-${ARCH}.zip"; \
      mkdir -p "/protoc/${ver}"; \
      curl -fsSL -o /tmp/protoc.zip "$url"; \
      unzip -q /tmp/protoc.zip -d "/protoc/${ver}"; \
      rm /tmp/protoc.zip; \
      "/protoc/${ver}/bin/protoc" --version; \
    done

# ── Stage 2: Build protoc-gen-buffa ────────────────────────────────────────

FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN cargo build --release --bin protoc-gen-buffa

# ── Stage 3: Run compatibility tests ──────────────────────────────────────

FROM debian:bookworm-slim

ARG PROTOC_VERSIONS="21.12 22.5 25.5 27.3 29.5 33.5"
ENV PROTOC_VERSIONS="$PROTOC_VERSIONS"

# Copy protoc binaries.
COPY --from=fetcher /protoc /protoc

# Copy the built plugin.
COPY --from=builder /src/target/release/protoc-gen-buffa /usr/local/bin/protoc-gen-buffa

# Copy test proto files.
COPY buffa-test/protos/ /test/protos/

# Copy the test script.
COPY compat/run-compat.sh /run-compat.sh
COPY compat/test-run-compat.sh /test-run-compat.sh
RUN chmod +x /run-compat.sh /test-run-compat.sh && /test-run-compat.sh

CMD ["/run-compat.sh"]

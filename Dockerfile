FROM rust:1.88-bookworm AS builder

WORKDIR /build

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/sp-core/Cargo.toml crates/sp-core/Cargo.toml
COPY crates/sp-analysis/Cargo.toml crates/sp-analysis/Cargo.toml
COPY crates/sp-db/Cargo.toml crates/sp-db/Cargo.toml
COPY crates/sp-registry-pypi/Cargo.toml crates/sp-registry-pypi/Cargo.toml
COPY crates/sp-server/Cargo.toml crates/sp-server/Cargo.toml
COPY crates/sp-client/Cargo.toml crates/sp-client/Cargo.toml

# Create dummy source files to build dependencies
RUN mkdir -p crates/sp-core/src && echo "" > crates/sp-core/src/lib.rs && \
    mkdir -p crates/sp-analysis/src && echo "" > crates/sp-analysis/src/lib.rs && \
    mkdir -p crates/sp-db/src && echo "" > crates/sp-db/src/lib.rs && \
    mkdir -p crates/sp-registry-pypi/src && echo "" > crates/sp-registry-pypi/src/lib.rs && \
    mkdir -p crates/sp-server/src && echo "fn main() {}" > crates/sp-server/src/main.rs && \
    mkdir -p crates/sp-client/src && echo "fn main() {}" > crates/sp-client/src/main.rs && \
    mkdir -p migrations && touch migrations/001_initial.sql

# Build dependencies (cached layer)
RUN cargo build --release 2>/dev/null || true

# Copy real source
COPY crates/ crates/
COPY migrations/ migrations/
COPY config/ config/
COPY skills/ skills/

# Build for real
RUN cargo build --release --bin secure-packages --bin sp-client

# ── Runtime ──
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl git && \
    rm -rf /var/lib/apt/lists/*

# Install Node.js 22 (Gemini CLI requires Node 20+)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -y nodejs && \
    rm -rf /var/lib/apt/lists/*

# Install Gemini CLI globally
RUN npm install -g @google/gemini-cli

COPY --from=builder /build/target/release/secure-packages /usr/local/bin/
COPY --from=builder /build/target/release/sp-client /usr/local/bin/
COPY --from=builder /build/config/ /app/config/
COPY --from=builder /build/skills/ /app/skills/

# Entrypoint script to ensure data dirs exist at runtime
RUN printf '#!/bin/sh\nmkdir -p /app/data/cache /app/data/tmp\nexec "$@"\n' > /app/entrypoint.sh && \
    chmod +x /app/entrypoint.sh

WORKDIR /app

EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

ENTRYPOINT ["/app/entrypoint.sh", "secure-packages"]
CMD ["serve"]

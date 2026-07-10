# ─────────────────────────────────────────────────────────────
# Stage 1: Frontend build
# ─────────────────────────────────────────────────────────────
FROM node:22-slim AS frontend

WORKDIR /app/web
COPY web/package*.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npx vite build

# ─────────────────────────────────────────────────────────────
# Stage 2: Backend build
# ─────────────────────────────────────────────────────────────
FROM rust:1-bookworm AS backend

WORKDIR /app
ARG NASFILES_BUILD_COMMIT=""
ARG NASFILES_BUILD_DATE=""

# Copy manifests first for better layer caching
COPY Cargo.toml Cargo.lock ./
COPY build-info.env build-info.env
COPY crates/nasfiles-core/Cargo.toml crates/nasfiles-core/Cargo.toml
COPY crates/nasfiles-server/Cargo.toml crates/nasfiles-server/Cargo.toml
COPY crates/nasfiles-server/build.rs crates/nasfiles-server/build.rs

# Create stub source files to pre-build dependencies, then remove stubs
# (leaves behind compiled dep artifacts for Cargo to reuse)
RUN mkdir -p crates/nasfiles-core/src crates/nasfiles-server/src && \
    echo 'fn main() {}' > crates/nasfiles-server/src/main.rs && \
    echo '' > crates/nasfiles-core/src/lib.rs && \
    cargo build --release 2>&1 | tail -5 && \
    rm -f target/release/deps/nasfiles* target/release/nasfiles* \
          target/release/deps/libnasfiles_core* target/release/libnasfiles_core* && \
    rm -rf crates/

# Copy real source code + migrations + frontend assets
COPY crates/ crates/
COPY migrations/ migrations/
COPY --from=frontend /app/web/dist web/dist

# Build the real binary
RUN NASFILES_BUILD_COMMIT="$NASFILES_BUILD_COMMIT" \
    NASFILES_BUILD_DATE="$NASFILES_BUILD_DATE" \
    cargo build --release --bin nasfiles

# ─────────────────────────────────────────────────────────────
# Stage 3: Runtime
# ─────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        ffmpeg \
        poppler-utils \
        libraw-bin \
        fonts-dejavu-core \
        p7zip-full \
        unar \
        bzip2 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r nasfiles && useradd -r -g nasfiles -s /sbin/nologin nasfiles

COPY --from=backend /app/target/release/nasfiles /usr/local/bin/nasfiles

# Default data directory
RUN mkdir -p /data && chown nasfiles:nasfiles /data

USER nasfiles
WORKDIR /data

ENV BIND_ADDR=0.0.0.0:8080 \
    DATA_DIR=/data \
    DB_URL=sqlite:///data/nasfiles.db?mode=rwc

EXPOSE 8080

ENTRYPOINT ["nasfiles"]

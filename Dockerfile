# syntax=docker/dockerfile:1

# ---- build -----------------------------------------------------------------
FROM rust:1-slim-bookworm AS builder
WORKDIR /app

# SQLx checks queries against the committed `.sqlx/` metadata, so the image
# builds without a database.
ENV SQLX_OFFLINE=true

# Build dependencies first so they are cached until Cargo.toml/Cargo.lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo 'fn main() {}' > src/main.rs \
    && touch src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY . .
RUN touch src/main.rs src/lib.rs && cargo build --release --locked

# ---- runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin commitvault

COPY --from=builder /app/target/release/commitvault /usr/local/bin/commitvault

USER 10001
ENV APP_HOST=0.0.0.0 \
    APP_PORT=3000
EXPOSE 3000

# Migrations are embedded in the binary and run on startup.
ENTRYPOINT ["/usr/local/bin/commitvault"]
CMD ["serve"]

FROM rust:1.99-bookworm AS builder

WORKDIR /app


COPY Cargo.toml Cargo.lock ./
COPY server/Cargo.toml server/Cargo.toml
COPY migration/Cargo.toml migration/Cargo.toml

RUN mkdir -p server/server/src/bin migration/src && \
    echo "fn main() {}" > server/server//src/main.rs && \
    echo "" > server/server/src/lib.rs && \
    echo "fn main() {}" > server/server/src/bin/seed.rs && \
    echo "fn main() {}" > server/server/src/bin/probe.rs && \
    echo "" > migration/src/lib.rs && \
    echo "fn main() {}" > migration/src/main.rs && \
    cargo build --release --bin server --bin migration 2>/dev/null || true && \
    rm -rf server/src migration/src

COPY server server
COPY migration migration
COPY Cargo.toml Cargo.lock ./

RUN cargo build --release --bin server --bin migration


# Runtime: server
FROM debian:bookworm-slim AS server

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 1000 app
USER app
WORKDIR /home/app

COPY --from=builder /app/target/release/server /usr/local/bin/server

EXPOSE 8080

CMD ["server"]

# Runtime: migration
FROM debian:bookworm-slim AS migrate

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 1000 app
USER app
WORKDIR /home/app

COPY --from=builder /app/target/release/migration /usr/local/bin/migration

CMD ["migration", "up"]
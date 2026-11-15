# O Cargo.lock exige Rust >= 1.88 (mesmo valor de rust-version no Cargo.toml).
FROM rust:1.88-slim-bookworm AS builder

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY static ./static
COPY src ./src

RUN cargo build --release --locked

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home appuser

COPY --from=builder /app/target/release/rpg-email-verifier /usr/local/bin/rpg-email-verifier

USER appuser
ENV VERIFIER_HOST=0.0.0.0
ENV VERIFIER_PORT=8081
EXPOSE 8081

CMD ["rpg-email-verifier"]


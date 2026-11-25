# syntax=docker/dockerfile:1
FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
RUN cargo build --locked --release -p tsql-lite-server --bin tsql-lite-server --bin tsql-lite-healthcheck

FROM debian:bookworm-slim
RUN mkdir -p /var/lib/tsql-lite && chown 10001:10001 /var/lib/tsql-lite
COPY --from=builder /build/target/release/tsql-lite-server /usr/local/bin/tsql-lite-server
COPY --from=builder /build/target/release/tsql-lite-healthcheck /usr/local/bin/tsql-lite-healthcheck
ENV HOME=/var/lib/tsql-lite RUST_LOG=info
WORKDIR /var/lib/tsql-lite
USER 10001:10001
EXPOSE 1433
HEALTHCHECK --interval=3s --timeout=3s --start-period=5s --retries=20 CMD ["/usr/local/bin/tsql-lite-healthcheck"]
ENTRYPOINT ["/usr/local/bin/tsql-lite-server", "--host", "0.0.0.0", "--port", "1433", "--data-dir", "/var/lib/tsql-lite", "--tls-gen"]

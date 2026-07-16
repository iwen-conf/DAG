# Build and run sunmao-server (linux amd64). No LLM.
# Clear host HTTP_PROXY that points at 127.0.0.1 (broken inside build).
FROM rust:1.96-bookworm AS builder
ENV http_proxy= https_proxy= HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= all_proxy=
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p sunmao-server

# Same base as builder so we keep ca-certificates/openssl without apt in final stage.
FROM rust:1.96-bookworm
ENV http_proxy= https_proxy= HTTP_PROXY= HTTPS_PROXY= ALL_PROXY= all_proxy=
WORKDIR /app
COPY --from=builder /src/target/release/sunmao-server /app/sunmao-server
COPY --from=builder /src/crates/sunmao-server/static /app/static
ENV SUNMAO_UI_DIR=/app/static
ENV RUST_LOG=sunmao_server=info
EXPOSE 7420
ENTRYPOINT ["/app/sunmao-server"]
CMD ["--db", "postgres://novel:admin123456@127.0.0.1:15432/sunmao", "--listen", "0.0.0.0:7420", "--unsafe-expose"]

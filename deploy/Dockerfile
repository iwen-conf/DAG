# Build and run sunmao-server (linux amd64). No LLM.
FROM rust:1.96-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# workspace-only build
RUN cargo build --release -p sunmao-server

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /src/target/release/sunmao-server /app/sunmao-server
COPY --from=builder /src/crates/sunmao-server/static /app/static
ENV SUNMAO_UI_DIR=/app/static
ENV RUST_LOG=sunmao_server=info
EXPOSE 7420
ENTRYPOINT ["/app/sunmao-server"]
CMD ["--db", "postgres://novel:admin123456@127.0.0.1:15432/sunmao", "--listen", "0.0.0.0:7420", "--unsafe-expose"]

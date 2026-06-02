FROM rust:1.88-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev protobuf-compiler

COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY proto ./proto
COPY migrations ./migrations
COPY src ./src

# Aqui está a mágica do cache mount:
# Ele guarda permanentemente os pacotes baixados e os arquivos intermediários compilados
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin grpc_api && \
    cp /app/target/release/grpc_api /grpc_api

# Imagem final baseada em Debian (compatível com a compilação do builder)
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /grpc_api /usr/local/bin/grpc_api

EXPOSE 50051

CMD ["grpc_api"]

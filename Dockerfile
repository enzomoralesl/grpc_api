FROM rust:1.88-alpine AS builder

WORKDIR /app

RUN apk add --no-cache musl-dev openssl-dev pkgconfig

# Pre-build dependencies to improve cache hits.
COPY Cargo.toml ./
COPY build.rs ./
COPY proto ./proto
COPY migrations ./migrations
COPY src ./src

RUN cargo build --release --bin grpc_api

FROM alpine:3.21

RUN apk add --no-cache ca-certificates

WORKDIR /app

COPY --from=builder /app/target/release/grpc_api /usr/local/bin/grpc_api

EXPOSE 50051

CMD ["grpc_api"]

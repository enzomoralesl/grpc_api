# Run Guide

This document explains how to run the users CRUD gRPC API locally with Cargo, with Docker, how to use the Rust CLI client, and how to run integration tests.

## What changed in this version
- Added user roles (`admin`, `user`).
- First created user becomes `admin`, all others become `user`.
- `ListUsers` is now `admin`-only.
- Added a Rust CLI client binary (`src/bin/client.rs`).
- Added integration test covering auth, admin enforcement, and CRUD flow.

## Prerequisites
- Rust toolchain (`cargo`, `rustc`)
- Docker and Docker Compose
- Optional: `grpcurl`

## 1) Run Locally with Cargo

### Start Postgres
```bash
docker compose up -d db
```

If you changed Postgres image/locale settings or hit `sqlx` protocol errors related to non-UTF-8 messages, recreate the database volume once:
```bash
docker compose down -v
docker compose up -d db
```

### Configure environment
```bash
cp .env.example .env
```

Default local values in `.env`:
- `DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/grpc_api`
- `JWT_SECRET=change_me_in_prod`
- `JWT_EXP_HOURS=24`
- `GRPC_ADDR=0.0.0.0:50051`
- `RUST_LOG=info`

### Run the API
```bash
cargo run
```

The server starts on `0.0.0.0:50051` and runs migrations automatically.

## 2) Run with Docker Compose

This starts Postgres and the API in containers.

```bash
docker compose up --build
```

The gRPC API is available on `localhost:50051`.

## 3) Run with Docker image only

If Postgres is already running elsewhere:

```bash
docker build -t grpc-api .
docker run --rm \
  -p 50051:50051 \
  -e DATABASE_URL=postgres://postgres:postgres@host.docker.internal:5432/grpc_api \
  -e JWT_SECRET=change_me_in_prod \
  -e JWT_EXP_HOURS=24 \
  -e GRPC_ADDR=0.0.0.0:50051 \
  -e RUST_LOG=info \
  grpc-api
```

## 4) Rust CLI client usage (recommended)

Client binary: `src/bin/client.rs`

### Create first user (becomes admin)
```bash
cargo run --bin client -- create-user admin@example.com "Admin One" secret123
```

### Login as admin
```bash
cargo run --bin client -- login admin@example.com secret123
```

Copy `token=...` output.

### Create regular user
```bash
cargo run --bin client -- create-user bob@example.com "Bob User" secret123
```

### List users as admin (allowed)
```bash
cargo run --bin client -- list-users <ADMIN_TOKEN>
```

### Login as regular user and test admin restriction
```bash
cargo run --bin client -- login bob@example.com secret123
cargo run --bin client -- list-users <BOB_TOKEN>
```

Expected: `PermissionDenied` for Bob.

### Get / update / delete self
```bash
cargo run --bin client -- get-user <BOB_TOKEN> <BOB_USER_ID>
cargo run --bin client -- update-user <BOB_TOKEN> <BOB_USER_ID> "Bob Updated"
cargo run --bin client -- delete-user <BOB_TOKEN> <BOB_USER_ID>
```

Optional override endpoint:
```bash
GRPC_ENDPOINT=http://127.0.0.1:50051 cargo run --bin client -- list-users <ADMIN_TOKEN>
```

## 5) grpcurl usage (optional)

### Create user
```bash
grpcurl -plaintext -import-path proto -proto users.proto \
  -d '{"email":"alice@example.com","full_name":"Alice Doe","password":"secret123"}' \
  localhost:50051 users.UsersService/CreateUser
```

### Login
```bash
grpcurl -plaintext -import-path proto -proto users.proto \
  -d '{"email":"alice@example.com","password":"secret123"}' \
  localhost:50051 users.UsersService/Login
```

### List users (admin only)
```bash
grpcurl -plaintext -import-path proto -proto users.proto \
  -H "authorization: Bearer <TOKEN>" \
  -d '{}' \
  localhost:50051 users.UsersService/ListUsers
```

## 6) Run integration tests

The integration test file is `tests/integration_users.rs`.

### Start Postgres first
```bash
docker compose up -d db
```

### Run tests
```bash
cargo test
```

Optional explicit test DB URL:
```bash
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/grpc_api cargo test
```

Test coverage includes:
- first user gets admin role
- second user gets user role
- admin can list users
- user cannot list users
- get/update/delete self flow

## 7) Stop services
```bash
docker compose down
```

To also remove Postgres data volume:
```bash
docker compose down -v
```

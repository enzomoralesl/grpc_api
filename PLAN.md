# PLAN

## Goal
Build a production-style Rust gRPC API with persistence, validation, authentication, and tests.

## Suggested Stack
- `tokio`: async runtime that runs the server and handles concurrent requests.
- `tonic`: gRPC framework used to define servers and clients.
- `prost`: compiles `.proto` files into Rust types and handles encoding.
- `tracing`: structured logging and observability for requests and errors.
- `sqlx`: async database access for Postgres and migrations.
- `jsonwebtoken`: creates and verifies JWT tokens for authentication.

## Phases

### 1. Project Setup
- Create the Rust project structure
- Add gRPC and protobuf tooling
- Configure formatting, linting, and testing

### 2. API Design
- Use a `users CRUD` domain
- Define protobuf messages and service methods
- Design request, response, and error shapes

### 3. gRPC Server
- Implement the first tonic service
- Add a unary RPC
- Run the server locally and test it with a client or `grpcurl`

### 4. Persistence
- Add a Postgres schema and migrations
- Implement a repository layer
- Replace in-memory storage with database-backed storage

### 5. Auth and Validation
- Add request validation
- Add authentication and authorization
- Map domain errors to gRPC status codes

### 6. Testing
- Add unit tests for business logic
- Add integration tests for gRPC endpoints
- Add database-backed test setup

### 7. Hardening
- Add structured logging and tracing
- Add configuration management
- Add Docker support
- Add deployment notes

### 8. Docker Usage
- Build the image with `docker build -t grpc-api .`
- Run API + Postgres together with `docker compose up --build`
- For image-only runs, pass `DATABASE_URL` and auth env vars to `docker run`

## Deliverables
- Working gRPC API
- Generated Rust protobuf code
- Persistent storage
- Authenticated endpoints
- Tests and documentation

## Learning Checkpoints
- Understand protobuf syntax
- Understand tonic service implementation
- Understand async Rust server code
- Understand API error handling in gRPC
- Understand how to structure a production-ready backend

## Notes
- Keep the first version small and focused.
- Start with one service and a few RPCs before adding extra complexity.
- Current implementation target: users CRUD with JWT auth and Postgres persistence.

use std::{env, net::SocketAddr};

use argon2::{
    password_hash::{
        rand_core::OsRng, Error as PasswordHashError, PasswordHash, PasswordHasher,
        PasswordVerifier, SaltString,
    },
    Argon2,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

pub mod users {
    tonic::include_proto!("users");
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("users_descriptor");
}

use users::users_service_server::{UsersService, UsersServiceServer};
use users::{
    CreateUserRequest, CreateUserResponse, DeleteUserRequest, DeleteUserResponse, GetUserRequest,
    GetUserResponse, ListUsersRequest, ListUsersResponse, LoginRequest, LoginResponse,
    UpdateUserRequest, UpdateUserResponse, User,
};

const ROLE_ADMIN: &str = "admin";
const ROLE_USER: &str = "user";

#[derive(Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_exp_hours: i64,
    pub grpc_addr: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/grpc_api".to_string());
        let jwt_secret =
            env::var("JWT_SECRET").unwrap_or_else(|_| "change_me_in_prod".to_string());
        let jwt_exp_hours = env::var("JWT_EXP_HOURS")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(24);
        let grpc_addr = env::var("GRPC_ADDR").unwrap_or_else(|_| "0.0.0.0:50051".to_string());

        Self {
            database_url,
            jwt_secret,
            jwt_exp_hours,
            grpc_addr,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

#[derive(Clone)]
pub struct JwtManager {
    secret: String,
    exp_hours: i64,
}

impl JwtManager {
    pub fn new(secret: String, exp_hours: i64) -> Self {
        Self { secret, exp_hours }
    }

    fn create_token(&self, user_id: Uuid) -> Result<String, Status> {
        let exp_ts = (Utc::now() + Duration::hours(self.exp_hours)).timestamp();
        let exp = usize::try_from(exp_ts).map_err(|_| Status::internal("invalid jwt exp"))?;

        let claims = Claims {
            sub: user_id.to_string(),
            exp,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_bytes()),
        )
        .map_err(|_| Status::internal("failed to sign jwt"))
    }

    fn verify_token(&self, token: &str) -> Result<Uuid, Status> {
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| Status::unauthenticated("invalid auth token"))?;

        Uuid::parse_str(&data.claims.sub)
            .map_err(|_| Status::unauthenticated("invalid token subject"))
    }
}

#[derive(sqlx::FromRow, Clone)]
struct UserRow {
    id: Uuid,
    email: String,
    full_name: String,
    role: String,
    password_hash: String,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
pub struct UsersGrpcService {
    pool: PgPool,
    jwt: JwtManager,
}

impl UsersGrpcService {
    pub fn new(pool: PgPool, jwt: JwtManager) -> Self {
        Self { pool, jwt }
    }

    fn validate_email(email: &str) -> bool {
        let trimmed = email.trim();
        trimmed.contains('@') && trimmed.contains('.') && trimmed.len() >= 5
    }

    fn hash_password(password: &str) -> Result<String, Status> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|value| value.to_string())
            .map_err(|_| Status::internal("failed to hash password"))
    }

    fn verify_password(hash: &str, password: &str) -> Result<bool, Status> {
        let parsed_hash =
            PasswordHash::new(hash).map_err(|_| Status::internal("stored password hash is invalid"))?;
        match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
            Ok(_) => Ok(true),
            Err(PasswordHashError::Password) => Ok(false),
            Err(_) => Err(Status::internal("failed to verify password")),
        }
    }

    fn validate_create(req: &CreateUserRequest) -> Result<(), Status> {
        if !Self::validate_email(&req.email) {
            return Err(Status::invalid_argument("invalid email"));
        }
        if req.full_name.trim().len() < 2 {
            return Err(Status::invalid_argument("full_name must be at least 2 characters"));
        }
        if req.password.len() < 8 {
            return Err(Status::invalid_argument("password must be at least 8 characters"));
        }
        Ok(())
    }

    fn token_from_request<T>(request: &Request<T>) -> Result<&str, Status> {
        let header = request
            .metadata()
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("missing authorization metadata"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("authorization header is not valid utf-8"))?;

        let (scheme, token) = header
            .split_once(' ')
            .ok_or_else(|| Status::unauthenticated("invalid authorization format"))?;

        if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
            return Err(Status::unauthenticated("expected bearer token"));
        }

        Ok(token)
    }

    fn auth_user_id<T>(&self, request: &Request<T>) -> Result<Uuid, Status> {
        let token = Self::token_from_request(request)?;
        self.jwt.verify_token(token)
    }

    fn ensure_self_access(auth_user: Uuid, target_user: Uuid) -> Result<(), Status> {
        if auth_user != target_user {
            return Err(Status::permission_denied(
                "you can only access your own user resource",
            ));
        }
        Ok(())
    }

    async fn ensure_admin(&self, auth_user: Uuid) -> Result<(), Status> {
        let role = sqlx::query_scalar::<_, String>("SELECT role FROM users WHERE id = $1")
            .bind(auth_user)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| Status::internal("failed to load auth user role"))?
            .ok_or_else(|| Status::unauthenticated("auth user no longer exists"))?;

        if role != ROLE_ADMIN {
            return Err(Status::permission_denied("admin role required"));
        }

        Ok(())
    }

    fn user_to_proto(row: UserRow) -> User {
        User {
            id: row.id.to_string(),
            email: row.email,
            full_name: row.full_name,
            role: row.role,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }

    fn is_unique_violation(err: &sqlx::Error) -> bool {
        matches!(err, sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505"))
    }
}

#[tonic::async_trait]
impl UsersService for UsersGrpcService {
    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        let req = request.into_inner();
        Self::validate_create(&req)?;

        let id = Uuid::new_v4();
        let email = req.email.trim().to_lowercase();
        let full_name = req.full_name.trim().to_string();
        let password_hash = Self::hash_password(&req.password)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| Status::internal("failed to start transaction"))?;

        let users_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::BIGINT FROM users")
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| Status::internal("failed to count users"))?;

        let role = if users_count == 0 { ROLE_ADMIN } else { ROLE_USER };

        let row = sqlx::query_as::<_, UserRow>(
            "INSERT INTO users (id, email, full_name, role, password_hash) VALUES ($1, $2, $3, $4, $5) RETURNING id, email, full_name, role, password_hash, created_at, updated_at",
        )
        .bind(id)
        .bind(email)
        .bind(full_name)
        .bind(role)
        .bind(password_hash)
        .fetch_one(&mut *tx)
        .await
        .map_err(|err| {
            if Self::is_unique_violation(&err) {
                Status::already_exists("email already exists")
            } else {
                Status::internal("failed to create user")
            }
        })?;

        tx.commit()
            .await
            .map_err(|_| Status::internal("failed to commit transaction"))?;

        Ok(Response::new(CreateUserResponse {
            user: Some(Self::user_to_proto(row)),
        }))
    }

    async fn login(&self, request: Request<LoginRequest>) -> Result<Response<LoginResponse>, Status> {
        let req = request.into_inner();
        if !Self::validate_email(&req.email) {
            return Err(Status::invalid_argument("invalid email"));
        }
        if req.password.is_empty() {
            return Err(Status::invalid_argument("password is required"));
        }

        let email = req.email.trim().to_lowercase();

        let user = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, full_name, role, password_hash, created_at, updated_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| Status::internal("failed to read user"))?
        .ok_or_else(|| Status::unauthenticated("invalid credentials"))?;

        let matches = Self::verify_password(&user.password_hash, &req.password)?;
        if !matches {
            return Err(Status::unauthenticated("invalid credentials"));
        }

        let token = self.jwt.create_token(user.id)?;
        Ok(Response::new(LoginResponse { token }))
    }

    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<GetUserResponse>, Status> {
        let auth_user = self.auth_user_id(&request)?;
        let req = request.into_inner();

        let user_id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("id must be a valid UUID"))?;
        Self::ensure_self_access(auth_user, user_id)?;

        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, full_name, role, password_hash, created_at, updated_at FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| Status::internal("failed to get user"))?
        .ok_or_else(|| Status::not_found("user not found"))?;

        Ok(Response::new(GetUserResponse {
            user: Some(Self::user_to_proto(row)),
        }))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let auth_user = self.auth_user_id(&request)?;
        self.ensure_admin(auth_user).await?;

        let rows = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, full_name, role, password_hash, created_at, updated_at FROM users ORDER BY created_at DESC LIMIT 100",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| Status::internal("failed to list users"))?;

        let users = rows.into_iter().map(Self::user_to_proto).collect();
        Ok(Response::new(ListUsersResponse { users }))
    }

    async fn update_user(
        &self,
        request: Request<UpdateUserRequest>,
    ) -> Result<Response<UpdateUserResponse>, Status> {
        let auth_user = self.auth_user_id(&request)?;
        let req = request.into_inner();

        let user_id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("id must be a valid UUID"))?;
        Self::ensure_self_access(auth_user, user_id)?;

        let existing = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, full_name, role, password_hash, created_at, updated_at FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| Status::internal("failed to read existing user"))?
        .ok_or_else(|| Status::not_found("user not found"))?;

        let email = if req.email.trim().is_empty() {
            existing.email.clone()
        } else {
            if !Self::validate_email(&req.email) {
                return Err(Status::invalid_argument("invalid email"));
            }
            req.email.trim().to_lowercase()
        };

        let full_name = if req.full_name.trim().is_empty() {
            existing.full_name.clone()
        } else {
            if req.full_name.trim().len() < 2 {
                return Err(Status::invalid_argument("full_name must be at least 2 characters"));
            }
            req.full_name.trim().to_string()
        };

        let password_hash = if req.update_password {
            if req.password.len() < 8 {
                return Err(Status::invalid_argument("password must be at least 8 characters"));
            }
            Self::hash_password(&req.password)?
        } else {
            existing.password_hash
        };

        let row = sqlx::query_as::<_, UserRow>(
            "UPDATE users SET email = $2, full_name = $3, password_hash = $4, updated_at = NOW() WHERE id = $1 RETURNING id, email, full_name, role, password_hash, created_at, updated_at",
        )
        .bind(user_id)
        .bind(email)
        .bind(full_name)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if Self::is_unique_violation(&err) {
                Status::already_exists("email already exists")
            } else {
                Status::internal("failed to update user")
            }
        })?;

        Ok(Response::new(UpdateUserResponse {
            user: Some(Self::user_to_proto(row)),
        }))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        let auth_user = self.auth_user_id(&request)?;
        let req = request.into_inner();

        let user_id =
            Uuid::parse_str(&req.id).map_err(|_| Status::invalid_argument("id must be a valid UUID"))?;
        Self::ensure_self_access(auth_user, user_id)?;

        let result = sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(|_| Status::internal("failed to delete user"))?;

        if result.rows_affected() == 0 {
            return Err(Status::not_found("user not found"));
        }

        Ok(Response::new(DeleteUserResponse { deleted: true }))
    }
}

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

pub async fn run_server(config: AppConfig) -> Result<(), Box<dyn std::error::Error>> {
    if config.jwt_secret == "change_me_in_prod" {
        warn!("JWT_SECRET is using the default insecure value");
    }

    let pool = create_pool(&config.database_url).await?;
    run_migrations(&pool).await?;

    let service = UsersGrpcService::new(
        pool,
        JwtManager::new(config.jwt_secret.clone(), config.jwt_exp_hours),
    );

    let addr: SocketAddr = config.grpc_addr.parse()?;
    info!("starting users gRPC API on {addr}");

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(users::FILE_DESCRIPTOR_SET)
        .build()
        .expect("failed to create gRPC reflection service");

    Server::builder()
        .add_service(reflection_service)
        .add_service(UsersServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

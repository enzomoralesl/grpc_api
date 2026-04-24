use std::{env, error::Error, io, time::Duration};

use grpc_api::users::users_service_client::UsersServiceClient;
use grpc_api::users::users_service_server::UsersServiceServer;
use grpc_api::users::{
    CreateUserRequest, DeleteUserRequest, GetUserRequest, ListUsersRequest, LoginRequest,
    UpdateUserRequest,
};
use grpc_api::{create_pool, run_migrations, JwtManager, UsersGrpcService};
use tonic::{metadata::MetadataValue, transport::Server, Code, Request};

fn with_auth<T>(message: T, token: &str) -> Result<Request<T>, Box<dyn Error>> {
    let mut request = Request::new(message);
    let header: MetadataValue<_> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", header);
    Ok(request)
}

async fn connect_with_retry(
    endpoint: &str,
) -> Result<UsersServiceClient<tonic::transport::Channel>, Box<dyn Error>> {
    let mut last_error: Option<tonic::transport::Error> = None;

    for _ in 0..30 {
        match UsersServiceClient::connect(endpoint.to_string()).await {
            Ok(client) => return Ok(client),
            Err(err) => {
                last_error = Some(err);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    Err(io::Error::other(format!(
        "failed to connect to test server: {}",
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown connection error".to_string())
    ))
    .into())
}

#[tokio::test]
async fn users_crud_and_admin_enforcement_flow() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok();

    let database_url = env::var("TEST_DATABASE_URL")
        .or_else(|_| env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/grpc_api".to_string());

    let pool = create_pool(&database_url).await?;
    run_migrations(&pool).await?;
    sqlx::query("TRUNCATE TABLE users")
        .execute(&pool)
        .await?;

    let temp_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = temp_listener.local_addr()?;
    drop(temp_listener);

    let server_pool = pool.clone();
    let server_task = tokio::spawn(async move {
        let jwt = JwtManager::new("integration-test-secret".to_string(), 24);
        let service = UsersGrpcService::new(server_pool, jwt);

        Server::builder()
            .add_service(UsersServiceServer::new(service))
            .serve(addr)
            .await
    });

    let endpoint = format!("http://{addr}");
    let mut client = connect_with_retry(&endpoint).await?;

    let admin_user = client
        .create_user(Request::new(CreateUserRequest {
            email: "admin@example.com".to_string(),
            full_name: "Admin One".to_string(),
            password: "secret123".to_string(),
        }))
        .await?
        .into_inner()
        .user
        .ok_or_else(|| io::Error::other("missing admin user"))?;
    assert_eq!(admin_user.role, "admin");

    let admin_token = client
        .login(Request::new(LoginRequest {
            email: "admin@example.com".to_string(),
            password: "secret123".to_string(),
        }))
        .await?
        .into_inner()
        .token;

    let regular_user = client
        .create_user(Request::new(CreateUserRequest {
            email: "bob@example.com".to_string(),
            full_name: "Bob User".to_string(),
            password: "secret123".to_string(),
        }))
        .await?
        .into_inner()
        .user
        .ok_or_else(|| io::Error::other("missing regular user"))?;
    assert_eq!(regular_user.role, "user");

    let regular_token = client
        .login(Request::new(LoginRequest {
            email: "bob@example.com".to_string(),
            password: "secret123".to_string(),
        }))
        .await?
        .into_inner()
        .token;

    let list_as_regular = client
        .list_users(with_auth(ListUsersRequest {}, &regular_token)?)
        .await
        .expect_err("regular user must not list users");
    assert_eq!(list_as_regular.code(), Code::PermissionDenied);

    let list_as_admin = client
        .list_users(with_auth(ListUsersRequest {}, &admin_token)?)
        .await?
        .into_inner();
    assert!(list_as_admin.users.len() >= 2);

    let fetched_user = client
        .get_user(with_auth(
            GetUserRequest {
                id: regular_user.id.clone(),
            },
            &regular_token,
        )?)
        .await?
        .into_inner()
        .user
        .ok_or_else(|| io::Error::other("missing fetched user"))?;
    assert_eq!(fetched_user.id, regular_user.id);

    let updated_user = client
        .update_user(with_auth(
            UpdateUserRequest {
                id: regular_user.id.clone(),
                email: "".to_string(),
                full_name: "Bob Updated".to_string(),
                password: "".to_string(),
                update_password: false,
            },
            &regular_token,
        )?)
        .await?
        .into_inner()
        .user
        .ok_or_else(|| io::Error::other("missing updated user"))?;
    assert_eq!(updated_user.full_name, "Bob Updated");

    let deleted = client
        .delete_user(with_auth(
            DeleteUserRequest {
                id: regular_user.id,
            },
            &regular_token,
        )?)
        .await?
        .into_inner();
    assert!(deleted.deleted);

    server_task.abort();
    let _ = server_task.await;

    Ok(())
}

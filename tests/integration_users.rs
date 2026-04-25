use std::{error::Error, io, time::Duration};

use grpc_api::users::users_service_client::UsersServiceClient;
use grpc_api::users::users_service_server::UsersServiceServer;
use grpc_api::users::{
    CreateUserRequest, ListUsersRequest, LoginRequest, UpdateUserRequest, User,
};
use grpc_api::{create_pool, run_migrations, JwtManager, UsersGrpcService};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
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

struct TestContext {
    pub client: UsersServiceClient<tonic::transport::Channel>,
    _container: testcontainers::ContainerAsync<Postgres>,
    _server_task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
}

impl TestContext {
    pub async fn create_and_login_user(&mut self, prefix: &str) -> Result<(User, String), Box<dyn Error>> {
        let email = format!("{prefix}@example.com");
        let password = "password123".to_string();

        let user = self.client.create_user(Request::new(CreateUserRequest {
            email: email.clone(),
            full_name: format!("{prefix} User"),
            password: password.clone(),
        })).await?.into_inner().user.unwrap();

        let token = self.client.login(Request::new(LoginRequest {
            email,
            password,
        })).await?.into_inner().token;

        Ok((user, token))
    }
}

async fn setup_test_context() -> Result<TestContext, Box<dyn Error>> {
    dotenvy::dotenv().ok();

    let container = Postgres::default().start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let pool = create_pool(&database_url).await?;
    run_migrations(&pool).await?;

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
    let client = connect_with_retry(&endpoint).await?;

    Ok(TestContext {
        client,
        _container: container,
        _server_task: server_task,
    })
}

#[tokio::test]
async fn test_user_cannot_list_users_permission_denied() -> Result<(), Box<dyn Error>> {
    let mut ctx = setup_test_context().await?;

    // The first user created in the system often becomes admin, but let's assume we create an admin first, then a regular user.
    let _admin = ctx.create_and_login_user("admin").await?;
    let (_user, user_token) = ctx.create_and_login_user("regular").await?;

    let res = ctx.client.list_users(with_auth(ListUsersRequest {}, &user_token)?).await;
    
    let err = res.unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    Ok(())
}

#[tokio::test]
async fn test_admin_can_list_users() -> Result<(), Box<dyn Error>> {
    let mut ctx = setup_test_context().await?;

    let (admin, admin_token) = ctx.create_and_login_user("admin").await?;

    let res = ctx.client.list_users(with_auth(ListUsersRequest {}, &admin_token)?).await;
    
    assert!(res.is_ok());
    let list = res.unwrap().into_inner();
    assert_eq!(list.users.len(), 1);
    assert_eq!(list.users[0].id, admin.id);

    Ok(())
}

#[tokio::test]
async fn test_user_can_update_own_profile() -> Result<(), Box<dyn Error>> {
    let mut ctx = setup_test_context().await?;

    let _admin = ctx.create_and_login_user("admin").await?;
    let (user, user_token) = ctx.create_and_login_user("regular").await?;

    let res = ctx.client.update_user(with_auth(UpdateUserRequest {
        id: user.id.clone(),
        email: "newemail@example.com".to_string(),
        full_name: "Updated Name".to_string(),
        password: "".to_string(),
        update_password: false,
    }, &user_token)?).await;

    assert!(res.is_ok());
    let updated = res.unwrap().into_inner().user.unwrap();
    assert_eq!(updated.full_name, "Updated Name");

    Ok(())
}

#[tokio::test]
async fn test_user_cannot_update_other_profiles() -> Result<(), Box<dyn Error>> {
    let mut ctx = setup_test_context().await?;

    let (admin, _) = ctx.create_and_login_user("admin").await?;
    let (_, user_token) = ctx.create_and_login_user("regular").await?;

    // Try to update admin's profile with regular user's token
    let res = ctx.client.update_user(with_auth(UpdateUserRequest {
        id: admin.id.clone(),
        email: "hacked@example.com".to_string(),
        full_name: "Hacked Admin".to_string(),
        password: "".to_string(),
        update_password: false,
    }, &user_token)?).await;

    let err = res.unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    Ok(())
}

use std::{env, io};

use grpc_api::users::users_service_client::UsersServiceClient;
use grpc_api::users::{
    CreateUserRequest, DeleteUserRequest, GetUserRequest, ListUsersRequest, LoginRequest,
    UpdateUserRequest,
};
use tonic::{metadata::MetadataValue, Request};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

fn usage() -> &'static str {
    "Usage:
  cargo run --bin client -- create-user <email> <full_name> <password>
  cargo run --bin client -- login <email> <password>
  cargo run --bin client -- get-user <token> <user_id>
  cargo run --bin client -- list-users <token>
  cargo run --bin client -- update-user <token> <user_id> <full_name> [email_or__] [password_or__]
  cargo run --bin client -- delete-user <token> <user_id>

Environment:
  GRPC_ENDPOINT=http://127.0.0.1:50051"
}

fn with_auth<T>(message: T, token: &str) -> Result<Request<T>, AnyError> {
    let mut request = Request::new(message);
    let header: MetadataValue<_> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", header);
    Ok(request)
}

fn grpc_endpoint() -> String {
    env::var("GRPC_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string())
}

async fn run() -> Result<(), AnyError> {
    let args: Vec<String> = env::args().collect();
    let Some(command) = args.get(1).map(String::as_str) else {
        println!("{}", usage());
        return Ok(());
    };

    let mut client = UsersServiceClient::connect(grpc_endpoint()).await?;

    match command {
        "create-user" => {
            if args.len() != 5 {
                return Err(io::Error::other(
                    "create-user requires: <email> <full_name> <password>",
                )
                .into());
            }
            let response = client
                .create_user(Request::new(CreateUserRequest {
                    email: args[2].clone(),
                    full_name: args[3].clone(),
                    password: args[4].clone(),
                }))
                .await?
                .into_inner();

            let user = response
                .user
                .ok_or_else(|| io::Error::other("server did not return user"))?;
            println!(
                "created: id={} email={} full_name={} role={}",
                user.id, user.email, user.full_name, user.role
            );
        }
        "login" => {
            if args.len() != 4 {
                return Err(io::Error::other("login requires: <email> <password>").into());
            }
            let response = client
                .login(Request::new(LoginRequest {
                    email: args[2].clone(),
                    password: args[3].clone(),
                }))
                .await?
                .into_inner();
            println!("token={}", response.token);
        }
        "get-user" => {
            if args.len() != 4 {
                return Err(io::Error::other("get-user requires: <token> <user_id>").into());
            }
            let response = client
                .get_user(with_auth(
                    GetUserRequest {
                        id: args[3].clone(),
                    },
                    &args[2],
                )?)
                .await?
                .into_inner();
            let user = response
                .user
                .ok_or_else(|| io::Error::other("server did not return user"))?;
            println!(
                "user: id={} email={} full_name={} role={} created_at={} updated_at={}",
                user.id, user.email, user.full_name, user.role, user.created_at, user.updated_at
            );
        }
        "list-users" => {
            if args.len() != 3 {
                return Err(io::Error::other("list-users requires: <token>").into());
            }
            let response = client
                .list_users(with_auth(ListUsersRequest {}, &args[2])?)
                .await?
                .into_inner();
            println!("users={}", response.users.len());
            for user in response.users {
                println!(
                    "- id={} email={} full_name={} role={}",
                    user.id, user.email, user.full_name, user.role
                );
            }
        }
        "update-user" => {
            if args.len() < 5 || args.len() > 7 {
                return Err(
                    io::Error::other(
                        "update-user requires: <token> <user_id> <full_name> [email_or_] [password_or_]",
                    )
                    .into(),
                );
            }

            let email = args.get(5).cloned().unwrap_or_else(|| "".to_string());
            let maybe_password = args.get(6).cloned().unwrap_or_else(|| "".to_string());

            let response = client
                .update_user(with_auth(
                    UpdateUserRequest {
                        id: args[3].clone(),
                        email: if email == "_" { "".to_string() } else { email },
                        full_name: args[4].clone(),
                        password: if maybe_password == "_" {
                            "".to_string()
                        } else {
                            maybe_password.clone()
                        },
                        update_password: !maybe_password.is_empty() && maybe_password != "_",
                    },
                    &args[2],
                )?)
                .await?
                .into_inner();
            let user = response
                .user
                .ok_or_else(|| io::Error::other("server did not return user"))?;
            println!(
                "updated: id={} email={} full_name={} role={}",
                user.id, user.email, user.full_name, user.role
            );
        }
        "delete-user" => {
            if args.len() != 4 {
                return Err(io::Error::other("delete-user requires: <token> <user_id>").into());
            }
            let response = client
                .delete_user(with_auth(
                    DeleteUserRequest {
                        id: args[3].clone(),
                    },
                    &args[2],
                )?)
                .await?
                .into_inner();
            println!("deleted={}", response.deleted);
        }
        _ => {
            println!("{}", usage());
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

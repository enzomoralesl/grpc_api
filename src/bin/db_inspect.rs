use std::env;
use sqlx::postgres::PgPoolOptions;
use chrono::Utc;
use uuid::Uuid;

#[derive(sqlx::FromRow, Debug)]
struct UserRow {
    id: Uuid,
    email: String,
    full_name: String,
    role: String,
    created_at: chrono::DateTime<Utc>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    
    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set in .env");

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await?;

    let admins = sqlx::query_as::<_, UserRow>(
        "SELECT id, email, full_name, role, created_at FROM users WHERE role = 'admin'"
    )
    .fetch_all(&pool)
    .await?;

    if admins.is_empty() {
        println!("Nenhum usuário com role 'admin' foi encontrado no banco de dados.");
    } else {
        println!("Usuários Admin encontrados no banco de dados:");
        for admin in admins {
            println!("- ID: {}", admin.id);
            println!("  Nome: {}", admin.full_name);
            println!("  Email: {}", admin.email);
            println!("  Criado em: {}", admin.created_at);
            println!(" Role: {}", admin.role);
            println!("--------------------------------------------------");
        }
    }

    Ok(())
}

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    pool: sqlx::PgPool,
    server_secret: String,
}

#[derive(Deserialize)]
struct CreateUserRequest {
    tg_id: i64,
}

#[derive(Serialize)]
struct CreateUserResponse {
    id: Uuid,
    tg_id: i64,
    uuid: Uuid,
}

#[derive(Serialize)]
struct SyncResponse {
    uuids: Vec<String>,
}

async fn create_user(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    let uuid = Uuid::new_v4();
    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO users (tg_id, uuid)
        VALUES ($1, $2)
        ON CONFLICT (tg_id) DO UPDATE SET updated_at = now()
        RETURNING id
        "#,
    )
    .bind(req.tg_id)
    .bind(uuid)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("create_user db error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "database error")
    })?;

    // Create a free 10-minute subscription for the user
    sqlx::query(
        r#"
        INSERT INTO subscriptions (user_id, plan_id, expire_date, status)
        VALUES ($1, $2, now() + interval '10 minutes', 'active')
        ON CONFLICT (user_id) DO UPDATE SET 
            expire_date = now() + interval '10 minutes',
            status = 'active',
            updated_at = now()
        "#,
    )
    .bind(id)
    .bind("free_trial")
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("create subscription db error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "database error")
    })?;

    let uuid: Uuid = sqlx::query_scalar("SELECT uuid FROM users WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "database error"))?;

    info!("user created/updated tg_id={} uuid={} with 10-minute free subscription", req.tg_id, uuid);
    Ok(Json(CreateUserResponse {
        id,
        tg_id: req.tg_id,
        uuid,
    }))
}

async fn sync(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    println!("Received sync request");
    let secret = headers
        .get("X-Server-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if secret != state.server_secret {
        return Err((StatusCode::UNAUTHORIZED, "invalid or missing X-Server-Secret"));
    }

    let rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT u.uuid FROM users u
        INNER JOIN subscriptions s ON s.user_id = u.id
        WHERE u.is_active = true
          AND s.status = 'active'
          AND s.expire_date > now()
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("sync db error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "database error")
    })?;

    let uuids: Vec<String> = rows.into_iter().map(|(u,)| u.to_string()).collect();
    println!("Sync returning {} active UUIDs", uuids.len());
    Ok(Json(SyncResponse { uuids }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let server_secret = std::env::var("SERVER_SECRET").unwrap_or_else(|_| String::new());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    let state = Arc::new(AppState {
        pool: pool.clone(),
        server_secret,
    });

    let app = Router::new()
        .route("/api/v1/users", post(create_user))
        .route("/api/internal/sync", get(sync))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], 3000));
    info!("control_plane listening on {}", addr);
    axum::serve(
        tokio::net::TcpListener::bind(addr).await?,
        app,
    )
    .await?;
    Ok(())
}

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Serialize;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tracing::info;
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    pool: sqlx::PgPool,
}

// The response now includes the Tariff Level (1, 2, 3, 4)
#[derive(Serialize)]
struct UserConfig {
    uuid: String,
    level: u32,
    email: String,
}

#[derive(Serialize)]
struct SyncResponse {
    users: Vec<UserConfig>,
}

async fn sync(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, &'static str)> {
    println!("Received sync request.");
    // 1. Get Secret
    println!("Headers: {:?}", headers);
    let secret = headers
        .get("X-Server-Secret")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing secret"))?;

    // 2. Identify Server by Secret
    let server_id: Uuid = sqlx::query_scalar("SELECT id FROM servers WHERE api_secret = $1")
        .bind(secret)
        .fetch_optional(&state.pool)
        .await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "db error"))?
        .ok_or((StatusCode::UNAUTHORIZED, "invalid secret"))?;

    // 3. Fetch Active Users assigned ONLY to THIS server
    // We join 'subscriptions' and 'tariffs' to get the xray_level
    let rows = sqlx::query_as::<_, (Uuid, i32, String)>(
        r#"
        SELECT 
            s.xray_uuid, 
            t.xray_level, 
            s.email 
        FROM subscriptions s
        JOIN tariffs t ON s.tariff_id = t.id
        WHERE s.server_id = $1 
          AND s.status = 'active'
          AND s.expire_date > now()
        "#,
    )
    .bind(server_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("sync db error: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "db error")
    })?;

    let users: Vec<UserConfig> = rows
        .into_iter()
        .map(|(uuid, level, email)| UserConfig {
            uuid: uuid.to_string(),
            level: level as u32,
            email,
        })
        .collect();

    info!("Server {} sync: {} active users", server_id, users.len());
    Ok(Json(SyncResponse { users }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt().init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let control_plane_url = std::env::var("CONTROL_PLANE_URL").expect("CONTROL_PLANE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    let state = Arc::new(AppState { pool });

    let app = Router::new()
        .route("/api/internal/sync", get(sync))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], control_plane_url
        .split(':')
        .last()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3333)));
    info!("Control Plane listening on {}", addr);
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

const SYNC_INTERVAL_SECS: u64 = 30;

#[derive(Deserialize)]
struct SyncResponse {
    uuids: Vec<String>,
}

/// Stub for future Xray gRPC client (add/remove users, reload config).
struct XrayClient {
    // TODO: gRPC channel, config path, etc.
}

impl XrayClient {
    fn new() -> Self {
        XrayClient {}
    }

    #[allow(dead_code)]
    fn add_user(&mut self, _uuid: &str) -> Result<()> {
        // TODO: call Xray gRPC to add user
        Ok(())
    }

    #[allow(dead_code)]
    fn remove_user(&mut self, _uuid: &str) -> Result<()> {
        // TODO: call Xray gRPC to remove user
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let control_plane_url =
        std::env::var("CONTROL_PLANE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
    let server_secret = std::env::var("SERVER_SECRET").expect("SERVER_SECRET must be set");

    let client = reqwest::Client::new();
    let mut xray = XrayClient::new();
    let mut local_uuids: HashSet<String> = HashSet::new();

    loop {
        match fetch_sync(&client, &control_plane_url, &server_secret).await {
            Ok(uuids) => {
                let remote: HashSet<String> = uuids.into_iter().collect();
                for uuid in &remote {
                    if !local_uuids.contains(uuid) {
                        println!("Adding user {}", uuid);
                        let _ = xray.add_user(uuid);
                        local_uuids.insert(uuid.clone());
                    }
                }
                for uuid in local_uuids.clone() {
                    if !remote.contains(&uuid) {
                        println!("Removing user {}", uuid);
                        let _ = xray.remove_user(&uuid);
                        local_uuids.remove(&uuid);
                    }
                }
            }
            Err(e) => {
                eprintln!("sync fetch error: {}", e);
            }
        }
        tokio::time::sleep(Duration::from_secs(SYNC_INTERVAL_SECS)).await;
    }
}

async fn fetch_sync(
    client: &reqwest::Client,
    base_url: &str,
    server_secret: &str,
) -> Result<Vec<String>> {
    let url = format!("{}/api/internal/sync", base_url.trim_end_matches('/'));
    let res = client
        .get(&url)
        .header("X-Server-Secret", server_secret)
        .send()
        .await?;
    anyhow::ensure!(res.status().is_success(), "sync returned {}", res.status());
    let body: SyncResponse = res.json().await?;
    Ok(body.uuids)
}

use anyhow::Result;
use prost::Message;
use prost::Name;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use xray_core::app::proxyman::command::{
    handler_service_client::HandlerServiceClient, AddUserOperation, AlterInboundRequest,
    RemoveUserOperation,
};
use xray_core::common::protocol::User;
use xray_core::common::serial::TypedMessage;
use xray_core::proxy::vless::Account;

const SYNC_INTERVAL_SECS: u64 = 30;
const XRAY_CONNECT_RETRY_SECS: u64 = 10;

/// Default inbound tag in Xray config (e.g. "inbound-vless").
const DEFAULT_INBOUND_TAG: &str = "inbound-vless";

#[derive(Deserialize)]
struct SyncResponse {
    uuids: Vec<String>,
}

/// Xray gRPC client for adding/removing users on an inbound (e.g. VLESS).
struct XrayClient {
    client: HandlerServiceClient<tonic::transport::Channel>,
    inbound_tag: String,
}

impl XrayClient {
    /// Connect to Xray gRPC API. `grpc_addr` can be e.g. "http://127.0.0.1:8080" or "https://...".
    async fn new(grpc_addr: &str, inbound_tag: Option<String>) -> Result<Self> {
        let client = HandlerServiceClient::connect(grpc_addr.to_string()).await?;
        Ok(XrayClient {
            client,
            inbound_tag: inbound_tag.unwrap_or_else(|| DEFAULT_INBOUND_TAG.to_string()),
        })
    }

    /// Add a VLESS user by UUID via AlterInbound + AddUserOperation.
    async fn add_user(&mut self, uuid: &str) -> Result<()> {
        let vless_account = Account {
            id: uuid.to_string(),
            flow: String::new(),
            encryption: "none".to_string(),
        };
        let account_typed = TypedMessage {
            r#type: Account::type_url(),
            value: vless_account.encode_to_vec(),
        };
        let user = User {
            level: 0,
            email: uuid.to_string(),
            account: Some(account_typed),
        };
        let op = AddUserOperation {
            user: Some(user),
        };
        let operation = TypedMessage {
            r#type: AddUserOperation::type_url(),
            value: op.encode_to_vec(),
        };
        let request = AlterInboundRequest {
            tag: self.inbound_tag.clone(),
            operation: Some(operation),
        };
        self.client
            .clone()
            .alter_inbound(tonic::Request::new(request))
            .await
            .map_err(|e| anyhow::anyhow!("xray alter_inbound add_user: {}", e))?;
        Ok(())
    }

    /// Remove a user by email (UUID) via AlterInbound + RemoveUserOperation.
    async fn remove_user(&mut self, uuid: &str) -> Result<()> {
        let op = RemoveUserOperation {
            email: uuid.to_string(),
        };
        let operation = TypedMessage {
            r#type: RemoveUserOperation::type_url(),
            value: op.encode_to_vec(),
        };
        let request = AlterInboundRequest {
            tag: self.inbound_tag.clone(),
            operation: Some(operation),
        };
        self.client
            .clone()
            .alter_inbound(tonic::Request::new(request))
            .await
            .map_err(|e| anyhow::anyhow!("xray alter_inbound remove_user: {}", e))?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let control_plane_url =
        std::env::var("CONTROL_PLANE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
    let server_secret = std::env::var("SERVER_SECRET").expect("SERVER_SECRET must be set");
    let grpc_addr = std::env::var("XRAY_GRPC_ADDR")
        .unwrap_or_else(|_| "http://host.docker.internal:8080".to_string());
    let inbound_tag = std::env::var("XRAY_INBOUND_TAG").ok();

    let client = reqwest::Client::new();
    let mut xray = loop {
        match XrayClient::new(&grpc_addr, inbound_tag.clone()).await {
            Ok(c) => break c,
            Err(e) => {
                eprintln!(
                    "Xray gRPC connect failed ({}), retrying in {}s: {}",
                    grpc_addr, XRAY_CONNECT_RETRY_SECS, e
                );
                tokio::time::sleep(Duration::from_secs(XRAY_CONNECT_RETRY_SECS)).await;
            }
        }
    };
    eprintln!("Connected to Xray gRPC at {}", grpc_addr);
    let mut local_uuids: HashSet<String> = HashSet::new();

    loop {
        match fetch_sync(&client, &control_plane_url, &server_secret).await {
            Ok(uuids) => {
                let remote: HashSet<String> = uuids.into_iter().collect();
                for uuid in &remote {
                    if !local_uuids.contains(uuid) {
                        println!("Adding user {}", uuid);
                        if let Err(e) = xray.add_user(uuid).await {
                            eprintln!("add_user {}: {}", uuid, e);
                        } else {
                            local_uuids.insert(uuid.clone());
                        }
                    }
                }
                for uuid in local_uuids.clone() {
                    if !remote.contains(&uuid) {
                        println!("Removing user {}", uuid);
                        if let Err(e) = xray.remove_user(&uuid).await {
                            eprintln!("remove_user {}: {}", uuid, e);
                        } else {
                            local_uuids.remove(&uuid);
                        }
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

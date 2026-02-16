use anyhow::Result;
use prost::Message;
use prost::Name; // Import Name trait to use type_url()
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

// Ensure your generated/imported modules match these paths
use xray_core::app::proxyman::command::{
    handler_service_client::HandlerServiceClient, AddUserOperation, AlterInboundRequest,
    RemoveUserOperation,
};
use xray_core::common::protocol::User;
use xray_core::common::serial::TypedMessage;
use xray_core::proxy::vless::Account;

const SYNC_INTERVAL_SECS: u64 = 30;
const XRAY_CONNECT_RETRY_SECS: u64 = 10;
const DEFAULT_INBOUND_TAG: &str = "inbound-vless";

#[derive(Deserialize)]
struct SyncResponse {
    uuids: Vec<String>,
}

struct XrayClient {
    client: HandlerServiceClient<Channel>,
    inbound_tag: String,
}

impl XrayClient {
    async fn new(grpc_addr: &str, inbound_tag: Option<String>) -> Result<Self> {
        let endpoint = Endpoint::from_shared(grpc_addr.to_string())?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(5))
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_while_idle(true)
            .http2_adaptive_window(true);

        let channel = endpoint.connect().await?;
        let client = HandlerServiceClient::new(channel);

        Ok(Self {
            client,
            inbound_tag: inbound_tag.unwrap_or_else(|| DEFAULT_INBOUND_TAG.to_string()),
        })
    }

    async fn add_user(&mut self, uuid: &str) -> Result<()> {
        // 1. Prepare VLESS Account
        let vless_account = Account {
            id: uuid.to_string(),
            flow: "xtls-rprx-vision".to_string(), // Use "xtls-rprx-vision" if using Reality+Vision
            encryption: "none".to_string(),
            ..Default::default() // Safely handle other fields if struct evolves
        };

        // FIX: Get clean type name without leading slash
        let account_type = Account::type_url();
        let account_type_clean = account_type.trim_start_matches('/');
        
        let account_typed = TypedMessage {
            r#type: account_type_clean.to_string(),
            value: vless_account.encode_to_vec(),
        };

        // 2. Prepare User
        let user = User {
            level: 0,
            email: uuid.to_string(),
            account: Some(account_typed),
        };

        // 3. Prepare Operation
        let op = AddUserOperation {
            user: Some(user),
        };

        // FIX: Get clean type name without leading slash
        let op_type = AddUserOperation::type_url();
        let op_type_clean = op_type.trim_start_matches('/');

        let operation = TypedMessage {
            r#type: op_type_clean.to_string(),
            value: op.encode_to_vec(),
        };

        let request = AlterInboundRequest {
            tag: self.inbound_tag.clone(),
            operation: Some(operation),
        };

        // DEBUG PRINT to verify the slash is gone
        println!(
            "DEBUG: Sending AddUser - OpType: '{}', AccType: '{}'",
            op_type_clean, account_type_clean
        );

        self.client
            .clone()
            .alter_inbound(tonic::Request::new(request))
            .await
            .map_err(|e| anyhow::anyhow!("xray alter_inbound add_user: {}", e))?;
        
        Ok(())
    }

    async fn remove_user(&mut self, uuid: &str) -> Result<()> {
        let op = RemoveUserOperation {
            email: uuid.to_string(),
        };

        // FIX: Get clean type name without leading slash
        let op_type = RemoveUserOperation::type_url();
        let op_type_clean = op_type.trim_start_matches('/');

        let operation = TypedMessage {
            r#type: op_type_clean.to_string(),
            value: op.encode_to_vec(),
        };

        let request = AlterInboundRequest {
            tag: self.inbound_tag.clone(),
            operation: Some(operation),
        };

        println!("DEBUG: Sending RemoveUser - OpType: '{}'", op_type_clean);

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
    let control_plane_url = std::env::var("CONTROL_PLANE_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
    let server_secret = std::env::var("SERVER_SECRET").expect("SERVER_SECRET must be set");
    let grpc_addr = std::env::var("XRAY_GRPC_ADDR").unwrap_or_else(|_| "http://host.docker.internal:8080".to_string());
    let inbound_tag = std::env::var("XRAY_INBOUND_TAG").ok();

    println!("Starting Proxy Agent...");
    println!("Connecting to gRPC at {}", grpc_addr);

    let client = reqwest::Client::new();
    let mut xray = loop {
        match XrayClient::new(&grpc_addr, inbound_tag.clone()).await {
            Ok(c) => break c,
            Err(e) => {
                eprintln!("Xray gRPC connect failed, retrying... ({})", e);
                tokio::time::sleep(Duration::from_secs(XRAY_CONNECT_RETRY_SECS)).await;
            }
        }
    };

    println!("Connected to Xray gRPC successfully.");
    let mut local_uuids: HashSet<String> = HashSet::new();

    loop {
        match fetch_sync(&client, &control_plane_url, &server_secret).await {
            Ok(uuids) => {
                let remote: HashSet<String> = uuids.into_iter().collect();

                // Add new users
                for uuid in &remote {
                    if !local_uuids.contains(uuid) {
                        println!("Processing ADD for user {}", uuid);
                        if let Err(e) = xray.add_user(uuid).await {
                            eprintln!("ERROR adding user {}: {}", uuid, e);
                        } else {
                            println!("SUCCESS added user {}", uuid);
                            local_uuids.insert(uuid.clone());
                        }
                    }
                }

                // Remove old users
                for uuid in local_uuids.clone() {
                    if !remote.contains(&uuid) {
                        println!("Processing REMOVE for user {}", uuid);
                        if let Err(e) = xray.remove_user(&uuid).await {
                            eprintln!("ERROR removing user {}: {}", uuid, e);
                        } else {
                            println!("SUCCESS removed user {}", uuid);
                            local_uuids.remove(&uuid);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Sync fetch error: {}", e);
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
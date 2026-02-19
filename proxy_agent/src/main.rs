use anyhow::Result;
use prost::Message;
use prost::Name; 
use serde::Deserialize;
use std::collections::{HashMap, HashSet}; // Use HashMap to track UUID -> Level
use std::time::Duration;
use tonic::transport::{Channel, Endpoint};

// Ensure your generated/imported modules match
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

// New Structure matches Control Plane
#[derive(Deserialize, Debug, Clone)]
struct UserConfig {
    uuid: String,
    level: u32,
    email: String,
}

#[derive(Deserialize)]
struct SyncResponse {
    users: Vec<UserConfig>,
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
            .keep_alive_while_idle(true);

        let channel = endpoint.connect().await?;
        let client = HandlerServiceClient::new(channel);
        Ok(Self {
            client,
            inbound_tag: inbound_tag.unwrap_or_else(|| DEFAULT_INBOUND_TAG.to_string()),
        })
    }

    async fn add_user(&mut self, user_cfg: &UserConfig) -> Result<()> {
        let vless_account = Account {
            id: user_cfg.uuid.clone(),
            flow: "xtls-rprx-vision".to_string(),
            encryption: "none".to_string(),
            ..Default::default()
        };

        let account_type = Account::type_url();
        let account_type_clean = account_type.trim_start_matches('/');
        
        let account_typed = TypedMessage {
            r#type: account_type_clean.to_string(),
            value: vless_account.encode_to_vec(),
        };

        let user = User {
            level: user_cfg.level, // Apply the Tariff Level Here
            email: user_cfg.email.clone(),
            account: Some(account_typed),
        };

        let op = AddUserOperation { user: Some(user) };
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

        self.client.alter_inbound(tonic::Request::new(request)).await?;
        Ok(())
    }

    async fn remove_user(&mut self, email: &str) -> Result<()> {
        let op = RemoveUserOperation { email: email.to_string() };
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

        self.client.alter_inbound(tonic::Request::new(request)).await?;
        Ok(())
    }
}

async fn fetch_sync(client: &reqwest::Client, base_url: &str, server_secret: &str) -> Result<Vec<UserConfig>> {
    let url = format!("{}/api/internal/sync", base_url.trim_end_matches('/'));
    println!("Fetching sync from Control Plane at {}", url);
    let res = client.get(&url).header("X-Server-Secret", server_secret).send().await?;
    anyhow::ensure!(res.status().is_success(), "sync returned {}", res.status());
    let body: SyncResponse = res.json().await?;
    Ok(body.users)
}

#[tokio::main]
async fn main() -> Result<()> {
    let control_plane_url = std::env::var("CONTROL_PLANE_URL").expect("CONTROL_PLANE_URL set");
    let server_secret = std::env::var("SERVER_SECRET").expect("SERVER_SECRET set");
    let grpc_addr = std::env::var("XRAY_GRPC_ADDR").unwrap_or_else(|_| "http://127.0.0.1:8080".into());
    let inbound_tag = std::env::var("XRAY_INBOUND_TAG").ok();

    println!("Starting Proxy Agent for Server...");

    // 1. Establish initial Xray connection
    let mut xray = loop {
        match XrayClient::new(&grpc_addr, inbound_tag.clone()).await {
            Ok(c) => break c,
            Err(_) => {
                eprintln!("Failed to connect to Xray at {}. Retrying in {} seconds...", grpc_addr, XRAY_CONNECT_RETRY_SECS);
                tokio::time::sleep(Duration::from_secs(XRAY_CONNECT_RETRY_SECS)).await;
            }
        }
    };
    
    println!("Connected to Xray at {}", grpc_addr);

    let http_client = reqwest::Client::new();
    
    // Track active users by Email (unique identifier in Xray)
    // We store the whole config to check if level changed later (optional optimization)
    let mut local_users: HashMap<String, UserConfig> = HashMap::new();

    loop {
        match fetch_sync(&http_client, &control_plane_url, &server_secret).await {
            Ok(remote_users_list) => {
                let mut remote_map: HashMap<String, UserConfig> = HashMap::new();
                
                // 1. Process Additions / Updates
                for cfg in remote_users_list {
                    remote_map.insert(cfg.email.clone(), cfg.clone());
                    
                    if !local_users.contains_key(&cfg.email) {
                        println!("Adding user: {} [Level {}]", cfg.email, cfg.level);
                        if let Err(e) = xray.add_user(&cfg).await {
                            eprintln!("Failed to add user {}: {}", cfg.email, e);
                        } else {
                            local_users.insert(cfg.email.clone(), cfg);
                        }
                    } 
                    // Optional: Check if level changed and update
                    // else if local_users[&cfg.email].level != cfg.level { ... }
                }

                // 2. Process Removals
                // We must clone keys to iterate while modifying
                let current_emails: Vec<String> = local_users.keys().cloned().collect();
                for email in current_emails {
                    if !remote_map.contains_key(&email) {
                        println!("Removing user: {}", email);
                        if let Err(e) = xray.remove_user(&email).await {
                            eprintln!("Failed to remove {}: {}", email, e);
                        } else {
                            local_users.remove(&email);
                        }
                    }
                }
            }
            Err(e) => eprintln!("Sync failed: {}", e),
        }
        tokio::time::sleep(Duration::from_secs(SYNC_INTERVAL_SECS)).await;
    }
}
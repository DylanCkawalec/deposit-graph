use crate::contracts::{deposit_graph, AppState};
use actix_web::web;
use anyhow::{Context, Result};
use ethers::middleware::SignerMiddleware;
use ethers::prelude::*;
use ethers::providers::{Http, Provider};
use ethers::signers::Signer;
use futures::StreamExt;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};



pub async fn listen_for_events(app_state: web::Data<Arc<AppState>>) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let processed_events_clone = app_state.processed_events.clone();
        let chain_id_clone = *chain_id;

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);

            loop {
                let withdrawal_filter = contract_clone
                    .withdrawal_requested_filter()
                    .from_block(BlockNumber::Latest);
                let shares_updated_filter = contract_clone
                    .shares_updated_filter()
                    .from_block(BlockNumber::Latest);
                let user_signed_up_filter = contract_clone
                    .user_signed_up_filter()
                    .from_block(BlockNumber::Latest);

                let withdrawal_stream = withdrawal_filter.stream().await;
                let shares_updated_stream = shares_updated_filter.stream().await;
                let user_signed_up_stream = user_signed_up_filter.stream().await;

                if let (Ok(mut w_stream), Ok(mut s_stream), Ok(mut u_stream)) = (
                    withdrawal_stream,
                    shares_updated_stream,
                    user_signed_up_stream,
                ) {
                    loop {
                        tokio::select! {
                            Some(Ok(event)) = w_stream.next() => {
                                info!("Received WithdrawalRequested event on chain {}", chain_id_clone);
                                if let Err(e) = process_withdrawal(event, &processed_events_clone, chain_id_clone).await {
                                    error!("Error processing withdrawal: {:?}", e);
                                }
                            }
                            Some(Ok(event)) = s_stream.next() => {
                                info!("Shares updated for user {:?}: {} on chain {}", event.user, event.new_shares, chain_id_clone);
                                if let Err(e) = process_shares_updated(event, chain_id_clone).await {
                                    error!("Error processing shares update: {:?}", e);
                                }
                            }
                            Some(Ok(event)) = u_stream.next() => {
                                info!("User signed up: {:?} on chain {}", event.user, chain_id_clone);
                                if let Err(e) = process_user_signed_up(event, chain_id_clone).await {
                                    error!("Error processing user sign up: {:?}", e);
                                }
                            }
                            else => break,
                        }
                    }
                } else {
                    error!("Failed to create event streams for chain ID: {}", chain_id_clone);
                }

                // Wait for a short period before restarting the loop
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }
    Ok(())
}

async fn process_withdrawal(
    withdrawal: deposit_graph::WithdrawalRequestedFilter,
    processed_events: &Arc<RwLock<HashMap<String, bool>>>,
    chain_id: U256,
) -> Result<()> {
    let event_id = format!(
        "withdrawal_{}_{}_{}_{}",
        chain_id, withdrawal.user, withdrawal.shares_withdrawn, withdrawal.token_amount
    );
    let mut events = processed_events.write().await;

    if events.contains_key(&event_id) {
        info!("Withdrawal already processed: {}", event_id);
        return Ok(());
    }

    info!(
        "Processing withdrawal for user {:?}, shares: {}, amount: {} on chain {}",
        withdrawal.user, withdrawal.shares_withdrawn, withdrawal.token_amount, chain_id
    );

    // Get the admin's private key from the environment
    let private_key = env::var("PRIVATE_KEY").context("PRIVATE_KEY must be set")?;
    let wallet: LocalWallet = private_key.parse().context("Failed to parse private key")?;

    // Get the RPC URL for the current chain
    let rpc_url = get_rpc_url_for_chain(chain_id)?;

    // Create a provider and connect the wallet
    let provider = Provider::<Http>::try_from(rpc_url)?;
    let client = SignerMiddleware::new(provider, wallet.clone().with_chain_id(chain_id.as_u64()));

    // Send tokens to the user
    let tx = client
        .send_transaction(
            TransactionRequest::new()
                .to(withdrawal.user)
                .value(withdrawal.token_amount)
                .from(wallet.address()),
            None,
        )
        .await?;

    info!("Sent withdrawal transaction: {:?}", tx);

    // Save event to file
    let event_json = json!({
        "event_type": "WithdrawalRequested",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", withdrawal.user),
        "shares_withdrawn": withdrawal.shares_withdrawn.to_string(),
        "token_amount": withdrawal.token_amount.to_string(),
        "contract_chain_id": withdrawal.chain_id.to_string(),
        "tx_hash": format!("{:?}", tx.tx_hash()),
    });

    save_event_to_file(&event_json, "events").await?;

    // Mark event as processed
    events.insert(event_id, true);

    // Move event file to processed folder
    move_event_to_processed(&event_json).await?;

    Ok(())
}

async fn process_shares_updated(event: deposit_graph::SharesUpdatedFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "SharesUpdated",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "new_shares": event.new_shares.to_string(),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await
}

async fn process_user_signed_up(event: deposit_graph::UserSignedUpFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "UserSignedUp",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await
}

fn get_rpc_url_for_chain(chain_id: U256) -> Result<String> {
    match chain_id.as_u64() {
        11155111 => env::var("ETHEREUM_SEPOLIA_RPC_URL").context("ETHEREUM_SEPOLIA_RPC_URL must be set"),
        84532 => env::var("BASE_SEPOLIA_RPC_URL").context("BASE_SEPOLIA_RPC_URL must be set"),
        11155420 => env::var("OPTIMISM_SEPOLIA_RPC_URL").context("OPTIMISM_SEPOLIA_RPC_URL must be set"),
        3441005 => env::var("MANTA_PACIFIC_SEPOLIA_RPC_URL").context("MANTA_PACIFIC_SEPOLIA_RPC_URL must be set"),
        _ => Err(anyhow::anyhow!("Unsupported chain ID: {}", chain_id)),
    }
}

async fn save_event_to_file(event_json: &serde_json::Value, folder: &str) -> Result<()> {
    let events_dir = format!("deposit-graph/{}", folder);
    fs::create_dir_all(&events_dir)?;

    let timestamp = chrono::Utc::now().timestamp_millis();
    let event_type = event_json["event_type"].as_str().unwrap_or("unknown");
    let user = event_json["user"].as_str().unwrap_or("unknown");
    
    let filename = format!("event_{}_{}_{}_{}.json", event_type, user, timestamp, fastrand::u64(..));
    let file_path = format!("{}/{}", events_dir, filename);

    fs::write(&file_path, serde_json::to_string_pretty(event_json)?)?;
    info!("Event saved to file: {}", file_path);

    Ok(())
}

async fn move_event_to_processed(event_json: &serde_json::Value) -> Result<()> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let event_type = event_json["event_type"].as_str().unwrap_or("unknown");
    let user = event_json["user"].as_str().unwrap_or("unknown");
    
    let filename = format!("event_{}_{}_{}_{}.json", event_type, user, timestamp, fastrand::u64(..));
    
    let source_path = format!("deposit-graph/events/{}", filename);
    let dest_path = format!("deposit-graph/events-finished/{}", filename);

    // Ensure the destination directory exists
    fs::create_dir_all("deposit-graph/events-finished")?;

    // Check if the source file exists before attempting to move it
    if fs::metadata(&source_path).is_ok() {
        fs::rename(&source_path, &dest_path)?;
        info!("Moved event file from {} to {}", source_path, dest_path);
    } else {
        warn!("Source file {} does not exist, skipping move operation", source_path);
    }

    Ok(())
}
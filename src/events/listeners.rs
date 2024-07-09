use crate::config::AppConfig;

use crate::contracts::{AppState, ContractType};
use actix_web::web;
use anyhow::{Context, Result};
use ethers::providers::Middleware;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use ethers::types::{TransactionRequest, U256};
use ethers::signers::Signer;
use ethers::middleware::SignerMiddleware;
use ethers::signers::LocalWallet;
use ethers::providers::{Provider, Http};
use crate::contracts::{
    WithdrawalRequestedFilter,
    SharesUpdatedFilter,
    UserSignedUpFilter,
};

async fn verify_chain_id<M: Middleware>(provider: &M, expected_chain_id: u64) -> Result<()>
where
    M::Error: 'static,
{
    let chain_id = provider.get_chainid().await?;
    if chain_id.as_u64() != expected_chain_id {
        return Err(anyhow::anyhow!(
            "Chain ID mismatch. Expected: {}, Got: {}",
            expected_chain_id,
            chain_id
        ));
    }
    Ok(())
}


pub async fn listen_for_events(app_state: web::Data<Arc<AppState>>, app_config: web::Data<AppConfig>) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let processed_events_clone = app_state.processed_events.clone();
        let chain_id_clone = *chain_id;
        let app_config_clone = app_config.clone();

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);

            let mut last_processed_block = match contract_clone.client().get_block_number().await {
                Ok(block) => block.as_u64(),
                Err(e) => {
                    error!("Failed to get initial block number for chain {}: {:?}", chain_id_clone, e);
                    return;
                }
            };

            info!("Initial block number for chain {}: {}", chain_id_clone, last_processed_block);

            loop {
                let latest_block = match contract_clone.client().get_block_number().await {
                    Ok(block) => block.as_u64(),
                    Err(e) => {
                        error!("Failed to get latest block for chain {}: {:?}", chain_id_clone, e);
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        continue;
                    }
                };

                info!("Latest block for chain {}: {}", chain_id_clone, latest_block);

                if latest_block > last_processed_block {
                    let from_block = last_processed_block + 1;
                    let to_block = latest_block;

                    info!("Processing blocks {} to {} for chain {}", from_block, to_block, chain_id_clone);

                    let withdrawal_filter = contract_clone
                        .withdrawal_requested_filter()
                        .from_block(from_block)
                        .to_block(to_block);
                    let shares_updated_filter = contract_clone
                        .shares_updated_filter()
                        .from_block(from_block)
                        .to_block(to_block);
                    let user_signed_up_filter = contract_clone
                        .user_signed_up_filter()
                        .from_block(from_block)
                        .to_block(to_block);

                    if let Err(e) = process_events(
                        &contract_clone,
                        withdrawal_filter,
                        shares_updated_filter,
                        user_signed_up_filter,
                        &processed_events_clone,
                        chain_id_clone,
                        &app_config_clone,
                    ).await {
                        error!("Error processing events for chain {}: {:?}", chain_id_clone, e);
                    }

                    info!("Finished processing blocks for chain {}", chain_id_clone);
                    last_processed_block = to_block;
                } else {
                    info!("No new blocks to process for chain {}", chain_id_clone);
                }

                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }
    Ok(())
}




async fn process_events<M: Middleware + 'static>(
    _contract: &Arc<ContractType>,
    withdrawal_filter: ethers::contract::Event<Arc<M>, M, WithdrawalRequestedFilter>,
    shares_updated_filter: ethers::contract::Event<Arc<M>, M, SharesUpdatedFilter>,
    user_signed_up_filter: ethers::contract::Event<Arc<M>, M, UserSignedUpFilter>,
    processed_events: &Arc<RwLock<HashMap<String, bool>>>,
    chain_id: U256,
    app_config: &AppConfig,
) -> Result<usize>  {
    let withdrawal_events = withdrawal_filter.query().await?;
    let shares_updated_events = shares_updated_filter.query().await?;
    let user_signed_up_events = user_signed_up_filter.query().await?;

    let mut event_count = 0;

    for event in withdrawal_events {
        info!("Received WithdrawalRequested event: {:?}", event);
        if let Err(e) = process_withdrawal(event, processed_events, chain_id, app_config).await {
            error!("Error processing withdrawal on chain {}: {:?}", chain_id, e);
        } else {
            event_count += 1;
        }
    }

    for event in shares_updated_events {
        if let Err(e) = process_shares_updated(event, chain_id).await {
            error!(
                "Error processing shares update on chain {}: {:?}",
                chain_id, e
            );
        } else {
            event_count += 1;
        }
    }

    for event in user_signed_up_events {
        if let Err(e) = process_user_signed_up(event, chain_id).await {
            error!(
                "Error processing user sign up on chain {}: {:?}",
                chain_id, e
            );
        } else {
            event_count += 1;
        }
    }

    Ok(event_count)
}

async fn process_withdrawal(
    withdrawal: WithdrawalRequestedFilter,
    processed_events: &Arc<RwLock<HashMap<String, bool>>>,
    chain_id: U256,
    app_config: &AppConfig,
) -> Result<()> {
    info!("Processing WithdrawalRequested event: {:?}", withdrawal);

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
    let rpc_url = app_config.get_rpc_url(chain_id.as_u64())?;
    info!("Using RPC URL: {}", rpc_url);

    // Create a provider and connect the wallet
    let provider = Provider::<Http>::try_from(rpc_url)?;
    let client = SignerMiddleware::new(provider, wallet.clone().with_chain_id(chain_id.as_u64()));

    // Verify the chain ID
    verify_chain_id(&client, chain_id.as_u64()).await?;

    info!("Wallet address: {:?}", wallet.address());

    // Check if the wallet has enough balance
    let balance = client.get_balance(wallet.address(), None).await?;
    if balance < withdrawal.token_amount {
        return Err(anyhow::anyhow!("Insufficient balance to process withdrawal"));
    }

    // Prepare the transaction
    let tx = TransactionRequest::new()
        .to(withdrawal.user)
        .value(withdrawal.token_amount)
        .from(wallet.address());

    // Send the transaction
    let pending_tx = client.send_transaction(tx, None).await?;
    let tx_hash = pending_tx.tx_hash();

    info!("Sent withdrawal transaction: {:?}", tx_hash);

    // Wait for the transaction to be mined
    let receipt = pending_tx.await?;

    if let Some(receipt) = receipt {
        info!("Transaction mined in block: {:?}", receipt.block_number);
    } else {
        warn!("Transaction failed or not mined: {:?}", tx_hash);
    }

    // Save event to file
    let event_json = json!({
        "event_type": "WithdrawalRequested",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", withdrawal.user),
        "shares_withdrawn": withdrawal.shares_withdrawn.to_string(),
        "token_amount": withdrawal.token_amount.to_string(),
        "contract_chain_id": withdrawal.chain_id.to_string(),
        "tx_hash": format!("{:?}", tx_hash),
    });

    save_event_to_file(&event_json, "events").await?;

    // Mark event as processed
    events.insert(event_id, true);

    // Move event file to processed folder
    move_event_to_processed(&event_json).await?;

    Ok(())
}


async fn process_shares_updated(event: SharesUpdatedFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "SharesUpdated",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "new_shares": event.new_shares.to_string(),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await
}

async fn process_user_signed_up(event: UserSignedUpFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "UserSignedUp",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await
}

// fn get_rpc_url_for_chain(chain_id: U256) -> Result<String> {
//     match chain_id.as_u64() {
//         11155111 => env::var("ETHEREUM_SEPOLIA_RPC_URL").context("ETHEREUM_SEPOLIA_RPC_URL must be set"),
//         84532 => env::var("BASE_SEPOLIA_RPC_URL").context("BASE_SEPOLIA_RPC_URL must be set"),
//         11155420 => env::var("OPTIMISM_SEPOLIA_RPC_URL").context("OPTIMISM_SEPOLIA_RPC_URL must be set"),
//         3441005 => env::var("MANTA_PACIFIC_SEPOLIA_RPC_URL").context("MANTA_PACIFIC_SEPOLIA_RPC_URL must be set"),
//         _ => Err(anyhow::anyhow!("Unsupported chain ID: {}", chain_id)),
//     }
// }

async fn save_event_to_file(event_json: &serde_json::Value, folder: &str) -> Result<()> {
    let events_dir = format!("deposit-graph/{}", folder);
    fs::create_dir_all(&events_dir)?;

    let timestamp = chrono::Utc::now().timestamp_millis();
    let event_type = event_json["event_type"].as_str().unwrap_or("unknown");
    let user = event_json["user"].as_str().unwrap_or("unknown");

    let filename = format!(
        "event_{}_{}_{}_{}.json",
        event_type,
        user,
        timestamp,
        fastrand::u64(..)
    );
    let file_path = format!("{}/{}", events_dir, filename);

    fs::write(&file_path, serde_json::to_string_pretty(event_json)?)?;
    info!("Event saved to file: {}", file_path);

    Ok(())
}

async fn move_event_to_processed(event_json: &serde_json::Value) -> Result<()> {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let event_type = event_json["event_type"].as_str().unwrap_or("unknown");
    let user = event_json["user"].as_str().unwrap_or("unknown");

    let filename = format!(
        "event_{}_{}_{}_{}.json",
        event_type,
        user,
        timestamp,
        fastrand::u64(..)
    );

    let source_path = format!("deposit-graph/events/{}", filename);
    let dest_path = format!("deposit-graph/events-finished/{}", filename);

    // Ensure the destination directory exists
    fs::create_dir_all("deposit-graph/events-finished")?;

    // Check if the source file exists before attempting to move it
    if fs::metadata(&source_path).is_ok() {
        fs::rename(&source_path, &dest_path)?;
        info!("Moved event file from {} to {}", source_path, dest_path);
    } else {
        error!(
            "Source file {} does not exist, skipping move operation",
            source_path
        );
    }

    Ok(())
}

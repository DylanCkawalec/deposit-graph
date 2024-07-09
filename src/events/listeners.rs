use crate::config::AppConfig;
use crate::contracts::{AppState, ContractType};
use crate::contracts::{
    DepositFilter, SharesUpdatedFilter, UserSignedUpFilter, WithdrawalRequestedFilter,
};
use actix_web::web;
use anyhow::{Context, Result};
use ethers::providers::Middleware;
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::ops::Deref;
use std::sync::Arc;

use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Provider};
use ethers::signers::LocalWallet;
use ethers::signers::Signer;
use ethers::types::{TransactionRequest, U256};
use tokio::sync::RwLock;
use tracing::{error, info};

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

pub async fn listen_for_events(
    app_state: web::Data<Arc<AppState>>,
    app_config: web::Data<AppConfig>,
) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let processed_events_clone = app_state.processed_events.clone();
        let chain_id_clone = *chain_id;
        let app_config_clone = app_config.clone();

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);

            // Get the current block number when the listener starts
            let mut last_processed_block = match contract_clone.client().get_block_number().await {
                Ok(block) => block.as_u64(),
                Err(e) => {
                    error!(
                        "Failed to get initial block number for chain {}: {:?}",
                        chain_id_clone, e
                    );
                    return;
                }
            };

            info!(
                "Starting to listen from block {} for chain {}",
                last_processed_block, chain_id_clone
            );

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;

                // Verify chain ID
                if let Err(e) =
                    verify_chain_id(contract_clone.client().deref(), chain_id_clone.as_u64()).await
                {
                    error!(
                        "Chain ID verification failed for chain {}: {:?}",
                        chain_id_clone, e
                    );
                    continue;
                }

                let latest_block = match contract_clone.client().get_block_number().await {
                    Ok(block) => block.as_u64(),
                    Err(e) => {
                        error!(
                            "Failed to get latest block for chain {}: {:?}",
                            chain_id_clone, e
                        );
                        continue;
                    }
                };

                if latest_block > last_processed_block {
                    let from_block = last_processed_block + 1;
                    let to_block = latest_block;

                    info!(
                        "Processing blocks {} to {} for chain {}",
                        from_block, to_block, chain_id_clone
                    );

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
                    let deposit_filter = contract_clone
                        .deposit_filter()
                        .from_block(from_block)
                        .to_block(to_block);

                    match process_events(
                        &contract_clone,
                        withdrawal_filter,
                        shares_updated_filter,
                        user_signed_up_filter,
                        deposit_filter,
                        &processed_events_clone,
                        chain_id_clone,
                        &app_config_clone,
                    )
                    .await
                    {
                        Ok(event_count) => info!(
                            "Processed {} events for chain {}",
                            event_count, chain_id_clone
                        ),
                        Err(e) => error!(
                            "Error processing events for chain {}: {:?}",
                            chain_id_clone, e
                        ),
                    }

                    last_processed_block = to_block;
                } else {
                    info!("No new blocks to process for chain {}", chain_id_clone);
                }
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
    deposit_filter: ethers::contract::Event<Arc<M>, M, DepositFilter>,
    processed_events: &Arc<RwLock<HashMap<String, bool>>>,
    chain_id: U256,
    app_config: &AppConfig,
) -> Result<usize> {
    let withdrawal_events = withdrawal_filter.query().await.unwrap_or_default();
    let shares_updated_events = shares_updated_filter.query().await.unwrap_or_default();
    let user_signed_up_events = user_signed_up_filter.query().await.unwrap_or_default();
    let deposit_events = deposit_filter.query().await.unwrap_or_default();

    let mut event_count = 0;

    for event in withdrawal_events {
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

    for event in deposit_events {
        if let Err(e) = process_deposit(event, chain_id).await {
            error!("Error processing deposit on chain {}: {:?}", chain_id, e);
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
        error!("Transaction failed: {:?}", tx_hash);
    }

    // Save event to file and mark as processed
    let event_json = json!({
        "event_type": "WithdrawalRequested",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", withdrawal.user),
        "shares_withdrawn": withdrawal.shares_withdrawn.to_string(),
        "token_amount": withdrawal.token_amount.to_string(),
        "tx_hash": format!("{:?}", tx_hash),
    });

    save_event_to_file(&event_json, "events").await?;
    events.insert(event_id, true);

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

    save_event_to_file(&event_json, "events").await?;
    move_event_to_processed(&event_json).await
}

async fn process_user_signed_up(event: UserSignedUpFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "UserSignedUp",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await?;
    move_event_to_processed(&event_json).await
}

async fn process_deposit(event: DepositFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "Deposit",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "amount": event.amount.to_string(),
        "new_shares": event.new_shares.to_string(),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(&event_json, "events").await?;
    move_event_to_processed(&event_json).await
}

async fn save_event_to_file(event_json: &serde_json::Value, folder: &str) -> Result<()> {
    let current_dir = env::current_dir()?;
    let events_dir = current_dir.join("deposit-graph").join(folder);
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
    let file_path = events_dir.join(&filename);

    fs::write(&file_path, serde_json::to_string_pretty(event_json)?)?;
    info!("Event saved to file: {}", file_path.display());

    Ok(())
}

async fn move_event_to_processed(event_json: &serde_json::Value) -> Result<()> {
    let current_dir = env::current_dir()?;
    let events_dir = current_dir.join("deposit-graph").join("events");
    let processed_dir = current_dir.join("deposit-graph").join("events-finished");

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

    let source_path = events_dir.join(&filename);
    let dest_path = processed_dir.join(&filename);

    // Ensure the destination directory exists
    fs::create_dir_all(&processed_dir)?;

    // Check if the source file exists before attempting to move it
    if source_path.exists() {
        fs::rename(&source_path, &dest_path)?;
        info!(
            "Moved event file from {} to {}",
            source_path.display(),
            dest_path.display()
        );
    } else {
        error!(
            "Source file {} does not exist, skipping move operation",
            source_path.display()
        );
    }

    Ok(())
}

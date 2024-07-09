use crate::config::AppConfig;
use crate::contracts::{AppState, ContractType};
use crate::contracts::{
    DepositFilter, SharesUpdatedFilter, UserSignedUpFilter, WithdrawalRequestedFilter,
};
use std::path::PathBuf;
//use ethers::utils::rlp;
//use ethers::core::types::transaction::eip2718::TypedTransaction;
use actix_web::web;
use anyhow::{Context, Result};
use ethers::middleware::SignerMiddleware;
use ethers::providers::Middleware;
use ethers::providers::{Http, Provider};
use ethers::signers::LocalWallet;
use ethers::signers::Signer;
use ethers::types::{TransactionRequest, U256};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info};
//use std::str::FromStr;
use std::ops::Div;
use std::path::Path;

async fn process_withdrawal(
    withdrawal: WithdrawalRequestedFilter,
    chain_id: U256,
    app_config: &AppConfig,
) -> Result<()> {
    info!(
        "Processing withdrawal for user {:?}, shares: {}, amount: {} on chain {}",
        withdrawal.user, withdrawal.shares_withdrawn, withdrawal.token_amount, chain_id
    );

    let private_key = env::var("PRIVATE_KEY").context("PRIVATE_KEY must be set")?;
    let wallet: LocalWallet = private_key.parse().context("Failed to parse private key")?;

    let rpc_url = app_config.get_rpc_url(chain_id.as_u64())?;
    info!("Using RPC URL: {}", rpc_url);

    let provider = Provider::<Http>::try_from(rpc_url)?;
    let client = SignerMiddleware::new(provider, wallet.clone().with_chain_id(chain_id.as_u64()));

    // Estimate gas price
    let gas_price = client.get_gas_price().await?;
    let gas_price_with_buffer = gas_price
        .saturating_mul(U256::from(120))
        .div(U256::from(100)); // Add 20% buffer

    // Prepare the transaction
    let mut tx = TransactionRequest::new()
        .to(withdrawal.user)
        .value(withdrawal.token_amount)
        .from(wallet.address())
        .gas_price(gas_price_with_buffer);

    // Estimate gas limit
    let gas_limit = client.estimate_gas(&tx.clone().into(), None).await?;
    tx = tx.gas(gas_limit);

    // Send the transaction
    let pending_tx = client.send_transaction(tx, None).await?;
    let tx_hash = pending_tx.tx_hash();

    info!("Sent withdrawal transaction: {:?}", tx_hash);

    // Wait for the transaction to be mined
    let receipt = pending_tx.await?;

    if let Some(receipt) = receipt {
        info!("Transaction mined in block: {:?}", receipt.block_number);

        let event_json = json!({
            "event_type": "WithdrawalProcessed",
            "chain_id": chain_id.to_string(),
            "user": format!("{:?}", withdrawal.user),
            "shares_withdrawn": withdrawal.shares_withdrawn.to_string(),
            "token_amount": withdrawal.token_amount.to_string(),
            "tx_hash": format!("{:?}", tx_hash),
            "block_number": receipt.block_number.unwrap().to_string(),
        });

        let events_dir = PathBuf::from("deposit-graph").join("events");
        save_event_to_file(&event_json, &events_dir).await?;
    } else {
        error!("Transaction failed: {:?}", tx_hash);
        return Err(anyhow::anyhow!("Transaction failed"));
    }

    Ok(())
}

async fn save_event_to_file(event_json: &serde_json::Value, events_dir: &Path) -> Result<()> {
    fs::create_dir_all(events_dir)?;

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

    // Move the file to processed
    let processed_dir = events_dir.parent().unwrap().join("events-finished");
    move_event_to_processed(&file_path, &processed_dir).await?;

    Ok(())
}

async fn move_event_to_processed(source_path: &Path, processed_dir: &Path) -> Result<()> {
    fs::create_dir_all(processed_dir)?;

    let dest_path = processed_dir.join(source_path.file_name().unwrap());

    for _ in 0..5 {
        // Try 5 times
        match fs::rename(source_path, &dest_path) {
            Ok(_) => {
                info!(
                    "Moved event file from {} to {}",
                    source_path.display(),
                    dest_path.display()
                );
                return Ok(());
            }
            Err(e) => {
                error!("Error moving file: {:?}", e);
                sleep(Duration::from_millis(100)).await;
            }
        }
    }

    error!(
        "Failed to move file {} after multiple attempts",
        source_path.display()
    );
    Ok(())
}

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

    info!("Received {} withdrawal events", withdrawal_events.len());
    info!(
        "Received {} shares updated events",
        shares_updated_events.len()
    );
    info!(
        "Received {} user signed up events",
        user_signed_up_events.len()
    );
    info!("Received {} deposit events", deposit_events.len());

    let mut event_count = 0;

    for event in withdrawal_events {
        info!("Processing withdrawal event: {:?}", event);
        let event_id = format!(
            "withdrawal_{}_{}_{}_{}",
            chain_id, event.user, event.shares_withdrawn, event.token_amount
        );
        let mut events = processed_events.write().await;
        if !events.contains_key(&event_id) {
            if let Err(e) = process_withdrawal(event, chain_id, app_config).await {
                error!(
                    "Error processing withdrawal request on chain {}: {:?}",
                    chain_id, e
                );
            } else {
                events.insert(event_id, true);
                event_count += 1;
            }
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

async fn process_shares_updated(event: SharesUpdatedFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "SharesUpdated",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "new_shares": event.new_shares.to_string(),
        "contract_chain_id": event.chain_id.to_string(),
    });

    let events_dir = PathBuf::from("deposit-graph").join("events");
    save_event_to_file(&event_json, &events_dir).await
}

async fn process_user_signed_up(event: UserSignedUpFilter, chain_id: U256) -> Result<()> {
    let event_json = json!({
        "event_type": "UserSignedUp",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "contract_chain_id": event.chain_id.to_string(),
    });

    let events_dir = PathBuf::from("deposit-graph").join("events");
    save_event_to_file(&event_json, &events_dir).await
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

    let events_dir = PathBuf::from("deposit-graph").join("events");
    save_event_to_file(&event_json, &events_dir).await
}

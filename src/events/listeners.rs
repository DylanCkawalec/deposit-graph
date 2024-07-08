use crate::contracts::{deposit_graph, AppState};
use actix_web::web;
use anyhow::Result;
use chrono::Utc;
use ethers::prelude::*;
use futures::StreamExt;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

pub async fn listen_for_events(app_state: web::Data<Arc<AppState>>) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let processed_events_clone = app_state.processed_events.clone();
        let chain_id_clone = *chain_id;

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);

            let withdrawal_filter = contract_clone
                .withdrawal_requested_filter()
                .from_block(0u64);
            let shares_updated_filter = contract_clone.shares_updated_filter().from_block(0u64);

            let mut withdrawal_stream = withdrawal_filter
                .stream()
                .await
                .expect("Failed to create withdrawal event stream");
            let mut shares_updated_stream = shares_updated_filter
                .stream()
                .await
                .expect("Failed to create shares updated event stream");

            loop {
                tokio::select! {
                    Some(Ok(event)) = withdrawal_stream.next() => {
                        info!("Received WithdrawalRequested event on chain {}", chain_id_clone);
                        if let Err(e) = process_withdrawal(event, &processed_events_clone, chain_id_clone).await {
                            error!("Error processing withdrawal: {:?}", e);
                        }
                    }
                    Some(Ok(event)) = shares_updated_stream.next() => {
                        info!("Shares updated for user {:?}: {} on chain {}", event.user, event.new_shares, chain_id_clone);
                        if let Err(e) = process_shares_updated(event, chain_id_clone).await {
                            error!("Error processing shares update: {:?}", e);
                        }
                    }
                    else => break,
                }
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
        chain_id, withdrawal.user, withdrawal.shares_withdrawn, withdrawal.eth_amount
    );
    let mut events = processed_events.write().await;

    if events.contains_key(&event_id) {
        info!("Withdrawal already processed: {}", event_id);
        return Ok(());
    }

    info!(
        "Processing withdrawal for user {:?}, shares: {}, amount: {} on chain {}",
        withdrawal.user, withdrawal.shares_withdrawn, withdrawal.eth_amount, chain_id
    );

    let event_json = json!({
        "event_type": "WithdrawalRequested",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", withdrawal.user),
        "shares_withdrawn": withdrawal.shares_withdrawn.to_string(),
        "eth_amount": withdrawal.eth_amount.to_string(),
        "contract_chain_id": withdrawal.chain_id.to_string(),
    });

    save_event_to_file(event_json).await?;

    events.insert(event_id, true);

    Ok(())
}

async fn process_shares_updated(
    event: deposit_graph::SharesUpdatedFilter,
    chain_id: U256,
) -> Result<()> {
    info!(
        "SharesUpdated event on chain {}: User: {:?}, New Shares: {}, Chain ID: {}",
        chain_id, event.user, event.new_shares, event.chain_id
    );

    let event_json = json!({
        "event_type": "SharesUpdated",
        "chain_id": chain_id.to_string(),
        "user": format!("{:?}", event.user),
        "new_shares": event.new_shares.to_string(),
        "contract_chain_id": event.chain_id.to_string(),
    });

    save_event_to_file(event_json).await
}

async fn save_event_to_file(event_json: serde_json::Value) -> Result<()> {
    let events_dir = "events";
    fs::create_dir_all(events_dir)?;

    let filename = format!(
        "{}/event_{}.json",
        events_dir,
        Utc::now().timestamp_millis()
    );
    fs::write(&filename, event_json.to_string())?;
    info!("Event saved to file: {}", filename);

    Ok(())
}

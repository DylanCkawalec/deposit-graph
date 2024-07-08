use crate::contracts::{AppState, ContractType};
use actix_web::web;
use anyhow::Result;
use ethers::prelude::*;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use std::fs::OpenOptions;
use std::io::Write;
use chrono::Utc;

pub async fn listen_for_events(app_state: web::Data<Arc<AppState>>) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let chain_id_clone = *chain_id;

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);
            
            let events = contract_clone.events();
            let mut stream = events.stream().await.expect("Failed to create event stream");

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => {
                        match event {
                            ContractType::Events::SharesUpdatedFilter(update) => {
                                info!("SharesUpdated event: user {:?}, new shares: {}, chain ID: {}", 
                                      update.user, update.new_shares, update.chain_id);
                                log_event("SharesUpdated", &format!("User: {:?}, New Shares: {}, Chain ID: {}", 
                                          update.user, update.new_shares, update.chain_id));
                            },
                            ContractType::Events::WithdrawalRequestedFilter(withdrawal) => {
                                info!("WithdrawalRequested event: user {:?}, shares withdrawn: {}, ETH amount: {}, chain ID: {}", 
                                      withdrawal.user, withdrawal.shares_withdrawn, withdrawal.eth_amount, withdrawal.chain_id);
                                log_event("WithdrawalRequested", &format!("User: {:?}, Shares Withdrawn: {}, ETH Amount: {}, Chain ID: {}", 
                                          withdrawal.user, withdrawal.shares_withdrawn, withdrawal.eth_amount, withdrawal.chain_id));
                            },
                            ContractType::Events::ChainIdSetFilter(chain_id_set) => {
                                info!("ChainIdSet event: new chain ID: {}", chain_id_set.chain_id);
                                log_event("ChainIdSet", &format!("New Chain ID: {}", chain_id_set.chain_id));
                            },
                        }
                    },
                    Err(e) => error!("Error in event stream for chain {}: {:?}", chain_id_clone, e),
                }
            }
        });
    }
    Ok(())
}

fn log_event(event_type: &str, details: &str) {
    let timestamp = Utc::now().to_rfc3339();
    let log_entry = format!("[{}] {}: {}\n", timestamp, event_type, details);
    
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("event_log.txt")
        .expect("Failed to open log file");

    if let Err(e) = file.write_all(log_entry.as_bytes()) {
        error!("Failed to write to log file: {:?}", e);
    }
}
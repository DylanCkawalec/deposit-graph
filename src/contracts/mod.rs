pub mod deposit_graph;

use crate::config::AppConfig;
use anyhow::{Context, Result};
use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Provider},
    signers::{LocalWallet, Signer},
    types::U256,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub use deposit_graph::DepositGraph;

pub type ContractType = DepositGraph<SignerMiddleware<Provider<Http>, LocalWallet>>;

pub struct AppState {
    pub contracts: HashMap<U256, Arc<ContractType>>,
    pub processed_events: Arc<RwLock<HashMap<String, bool>>>,
    pub drpc_api_key: String,
}

pub async fn initialize_contracts(config: &AppConfig) -> Result<HashMap<U256, Arc<ContractType>>> {
    let mut contracts = HashMap::new();

    for chain_config in &config.chain_configs {
        println!(
            "Initializing contract for chain ID: {}",
            chain_config.chain_id
        );

        let rpc_url = config.get_rpc_url(chain_config.chain_id)?;
        let provider = Provider::<Http>::try_from(rpc_url)?;
        let wallet: LocalWallet = config.private_key.parse()?;
        let client = SignerMiddleware::new(provider, wallet.with_chain_id(chain_config.chain_id));

        println!(
            "Attempting to read environment variable: {}",
            chain_config.contract_address_env
        );
        let contract_address =
            std::env::var(&chain_config.contract_address_env).with_context(|| {
                format!(
                    "Missing environment variable: {}. All env vars: {:?}",
                    chain_config.contract_address_env,
                    std::env::vars().collect::<HashMap<_, _>>()
                )
            })?;
        println!(
            "Contract address for chain {}: {}",
            chain_config.chain_id, contract_address
        );

        let contract_address: ethers::types::Address = contract_address.parse()?;
        let contract = DepositGraph::new(contract_address, Arc::new(client));

        contracts.insert(U256::from(chain_config.chain_id), Arc::new(contract));
        println!(
            "Contract initialized for chain ID: {}",
            chain_config.chain_id
        );
    }

    Ok(contracts)
}

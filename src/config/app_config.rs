use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
//use std::collections::HashMap;



#[derive(Clone, Deserialize)]
pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub private_key: String,
    pub drpc_api_key: String,
    pub chain_configs: Vec<ChainConfig>,
}

#[derive(Clone, Deserialize)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub network: String,
    pub contract_address_env: String,
    pub rpc_url_env: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        println!("Loading configuration from environment variables");

        let config = Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .context("Failed to parse PORT")?,
            private_key: env::var("PRIVATE_KEY").context("PRIVATE_KEY must be set")?,
            drpc_api_key: env::var("DRPC_API_KEY").context("DRPC_API_KEY must be set")?,
            chain_configs: Self::load_chain_configs(),
        };

        println!("Chain configurations:");
        for chain_config in &config.chain_configs {
            println!(
                "Chain ID: {}, Network: {}, Contract Address Env: {}, RPC URL Env: {}",
                chain_config.chain_id, chain_config.network, chain_config.contract_address_env, chain_config.rpc_url_env
            );
            match env::var(&chain_config.contract_address_env) {
                Ok(address) => println!(
                    "  {} is set to: {}",
                    chain_config.contract_address_env, address
                ),
                Err(e) => println!(
                    "  Error reading {}: {:?}",
                    chain_config.contract_address_env, e
                ),
            }
            match env::var(&chain_config.rpc_url_env) {
                Ok(url) => println!(
                    "  {} is set to: {}",
                    chain_config.rpc_url_env, url
                ),
                Err(e) => println!(
                    "  Error reading {}: {:?}",
                    chain_config.rpc_url_env, e
                ),
            }
        }

        Ok(config)
    }

    fn load_chain_configs() -> Vec<ChainConfig> {
        vec![
            ChainConfig {
                chain_id: 11155111,
                network: "sepolia".to_string(),
                contract_address_env: "ETHEREUM_SEPOLIA_CONTRACT_ADDRESS".to_string(),
                rpc_url_env: "ETHEREUM_SEPOLIA_RPC_URL".to_string(),
            },
            ChainConfig {
                chain_id: 11155420,
                network: "optimism-sepolia".to_string(),
                contract_address_env: "OPTIMISM_SEPOLIA_CONTRACT_ADDRESS".to_string(),
                rpc_url_env: "OPTIMISM_SEPOLIA_RPC_URL".to_string(),
            },
            // Add more chain configs here as needed
        ]
    }

    pub fn get_rpc_url(&self, chain_id: u64) -> Result<String> {
        let chain_config = self.chain_configs.iter()
            .find(|config| config.chain_id == chain_id)
            .ok_or_else(|| anyhow::anyhow!("Unsupported chain ID: {}", chain_id))?;
        
        env::var(&chain_config.rpc_url_env)
            .context(format!("{} must be set", chain_config.rpc_url_env))
    }
}
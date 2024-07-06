use anyhow::Result;
use serde::Deserialize;
use std::env;
use std::path::Path;

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
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let env_paths = vec![".env", "../.env", "../../.env"];

        for path in env_paths {
            if Path::new(path).exists() {
                dotenv::from_path(path).ok();
                break;
            }
        }

        for (key, value) in std::env::vars() {
            println!("{}: {}", key, value);
        }

        Ok(Self {
            host: env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            private_key: env::var("PRIVATE_KEY")?,
            drpc_api_key: env::var("DRPC_API_KEY")?,
            chain_configs: vec![
                ChainConfig {
                    chain_id: 11155111,
                    network: "sepolia".to_string(),
                    contract_address_env: "ETHEREUM_SEPOLIA_CONTRACT_ADDRESS".to_string(),
                },
                // Commenting out other networks for now
                /*
                ChainConfig {
                    chain_id: 84532,
                    network: "base-sepolia".to_string(),
                    contract_address_env: "BASE_SEPOLIA_CONTRACT_ADDRESS".to_string(),
                },
                ChainConfig {
                    chain_id: 11155420,
                    network: "optimism-sepolia".to_string(),
                    contract_address_env: "OPTIMISM_SEPOLIA_CONTRACT_ADDRESS".to_string(),
                },
                ChainConfig {
                    chain_id: 3441005,
                    network: "manta-pacific-sepolia".to_string(),
                    contract_address_env: "MANTA_PACIFIC_CONTRACT_ADDRESS".to_string(),
                },
                */
            ],
        })
    }
}

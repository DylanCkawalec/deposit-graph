use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use anyhow::{Context, Result};
use ethers::{
    contract::abigen,
    core::types::{Address, U256},
    middleware::{Middleware, SignerMiddleware},
    providers::{Http, Provider, StreamExt},
    signers::{LocalWallet, Signer},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
use std::path::Path;

abigen!(
    DepositGraph,
    "../build/contracts/DepositGraph.json",
    event_derives(serde::Serialize, serde::Deserialize)
);

type ContractType = DepositGraph<SignerMiddleware<Provider<Http>, LocalWallet>>;

struct AppState {
    contracts: HashMap<U256, Arc<ContractType>>,
    processed_events: Arc<RwLock<HashMap<String, bool>>>,
    drpc_api_key: String,
}

#[derive(Deserialize)]
struct BlobUpdate {
    blob: String,
    chain_id: U256,
}

#[derive(Deserialize)]
struct UserAction {
    address: Address,
    chain_id: U256,
}

#[derive(Deserialize)]
struct Deposit {
    address: Address,
    amount: U256,
    chain_id: U256,
}

#[derive(Deserialize)]
struct Withdraw {
    address: Address,
    shares: U256,
    chain_id: U256,
}

#[derive(Serialize)]
struct ApiResponse {
    status: String,
    tx_hash: Option<String>,
    message: String,
    data: Option<serde_json::Value>,
}

async fn sign_up(user: web::Json<UserAction>, data: web::Data<AppState>) -> impl Responder {
    let contract = match data.contracts.get(&user.chain_id) {
        Some(c) => c,
        None => return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Unsupported chain ID: {}", user.chain_id),
            data: None,
        }),
    };

    match contract.sign_up().from(user.address).send().await {
        Ok(tx) => {
            let tx_hash = format!("{:?}", tx.tx_hash());
            HttpResponse::Ok().json(ApiResponse {
                status: "success".to_string(),
                tx_hash: Some(tx_hash),
                message: "User signed up successfully".to_string(),
                data: None,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error signing up: {:?}", e),
            data: None,
        }),
    }
}

async fn deposit(deposit: web::Json<Deposit>, data: web::Data<AppState>) -> impl Responder {
    let contract = match data.contracts.get(&deposit.chain_id) {
        Some(c) => c,
        None => return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Unsupported chain ID: {}", deposit.chain_id),
            data: None,
        }),
    };

    match contract.deposit().from(deposit.address).value(deposit.amount).send().await {
        Ok(tx) => {
            let tx_hash = format!("{:?}", tx.tx_hash());
            HttpResponse::Ok().json(ApiResponse {
                status: "success".to_string(),
                tx_hash: Some(tx_hash),
                message: "Deposit successful".to_string(),
                data: None,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error depositing: {:?}", e),
            data: None,
        }),
    }
}

async fn withdraw(withdraw: web::Json<Withdraw>, data: web::Data<AppState>) -> impl Responder {
    let contract = match data.contracts.get(&withdraw.chain_id) {
        Some(c) => c,
        None => return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Unsupported chain ID: {}", withdraw.chain_id),
            data: None,
        }),
    };

    match contract.withdraw(withdraw.shares).from(withdraw.address).send().await {
        Ok(tx) => {
            let tx_hash = format!("{:?}", tx.tx_hash());
            HttpResponse::Ok().json(ApiResponse {
                status: "success".to_string(),
                tx_hash: Some(tx_hash),
                message: "Withdrawal successful".to_string(),
                data: None,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error withdrawing: {:?}", e),
            data: None,
        }),
    }
}

async fn get_shares(user: web::Json<UserAction>, data: web::Data<AppState>) -> impl Responder {
    let contract = match data.contracts.get(&user.chain_id) {
        Some(c) => c,
        None => return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Unsupported chain ID: {}", user.chain_id),
            data: None,
        }),
    };

    match contract.shares(user.address).call().await {
        Ok(shares) => HttpResponse::Ok().json(ApiResponse {
            status: "success".to_string(),
            tx_hash: None,
            message: "Shares retrieved successfully".to_string(),
            data: Some(serde_json::json!({ "shares": shares })),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error getting shares: {:?}", e),
            data: None,
        }),
    }
}

async fn update_shares(blob: web::Json<BlobUpdate>, data: web::Data<AppState>) -> impl Responder {
    let contract = match data.contracts.get(&blob.chain_id) {
        Some(c) => c,
        None => return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Unsupported chain ID: {}", blob.chain_id),
            data: None,
        }),
    };

    let event_id = format!("blob_update_{}_{}", blob.chain_id, blob.blob);
    let mut events = data.processed_events.write().await;

    if events.contains_key(&event_id) {
        return HttpResponse::BadRequest().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: "Blob update already processed".to_string(),
            data: None,
        });
    }

    match contract.blob_update(blob.blob.clone()).send().await {
        Ok(tx) => {
            let tx_hash = format!("{:?}", tx.tx_hash());
            events.insert(event_id, true);
            HttpResponse::Ok().json(ApiResponse {
                status: "success".to_string(),
                tx_hash: Some(tx_hash),
                message: "Blob update processed successfully".to_string(),
                data: None,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error processing blob update: {:?}", e),
            data: None,
        }),
    }
}

async fn test_rpc(data: web::Data<AppState>) -> impl Responder {
    let url = format!(
        "https://lb.drpc.org/ogrpc?network=ethereum&dkey={}",
        data.drpc_api_key
    );
    let provider = Provider::<Http>::try_from(url).unwrap();

    match provider.get_block_number().await {
        Ok(block_number) => HttpResponse::Ok().json(ApiResponse {
            status: "success".to_string(),
            tx_hash: None,
            message: "Block number retrieved successfully".to_string(),
            data: Some(serde_json::json!({ "block_number": format!("0x{:x}", block_number) })),
        }),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Failed to get block number: {:?}", e),
            data: None,
        }),
    }
}

#[actix_web::main]
async fn main() -> Result<()> {
    let dot_env_path = Path::new("../../.env");
    dotenv::from_path(dot_env_path).ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .init();

    info!("Starting DepositGraph service");

    let private_key = env::var("PRIVATE_KEY").context("PRIVATE_KEY must be set")?;
    let drpc_api_key = env::var("DRPC_API_KEY").context("DRPC_API_KEY must be set")?;

    let mut contracts = HashMap::new();

    let chain_configs = vec![
        (U256::from(11155111), "sepolia", "ETHEREUM_SEPOLIA_CONTRACT_ADDRESS"),
        (U256::from(84532), "base-sepolia", "BASE_SEPOLIA_CONTRACT_ADDRESS"),
        (U256::from(11155420), "optimism-sepolia", "OPTIMISM_SEPOLIA_CONTRACT_ADDRESS"),
        (U256::from(3441005), "manta-pacific-sepolia", "MANTA_PACIFIC_CONTRACT_ADDRESS"),
    ];

    for (chain_id, network, contract_env_var) in chain_configs {
        info!("Initializing contract for chain ID: {}", chain_id);
        let rpc_url = format!(
            "https://lb.drpc.org/ogrpc?network={}&dkey={}",
            network, drpc_api_key
        );
        let provider = Provider::<Http>::try_from(rpc_url).context(format!(
            "Failed to connect to provider for chain ID: {}",
            chain_id
        ))?;
        let wallet: LocalWallet = private_key.parse().context("Failed to parse private key")?;
        let client = SignerMiddleware::new(provider, wallet.clone().with_chain_id(chain_id.as_u64()));

        let contract_address = env::var(contract_env_var).context(format!("{} must be set", contract_env_var))?;
        let contract_address: Address = contract_address
            .parse()
            .context("Failed to parse contract address")?;
        let contract = DepositGraph::new(contract_address, Arc::new(client));

        contracts.insert(chain_id, Arc::new(contract));
        info!("Contract initialized for chain ID: {}", chain_id);
    }

    let app_state = web::Data::new(AppState {
        contracts,
        processed_events: Arc::new(RwLock::new(HashMap::new())),
        drpc_api_key: drpc_api_key.clone(),
    });

    let app_state_clone = app_state.clone();

    tokio::spawn(async move {
        if let Err(e) = listen_for_events(app_state_clone).await {
            error!("Error in event listener: {:?}", e);
        }
    });

    info!("Starting HTTP server");
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .route("/sign_up", web::post().to(sign_up))
            .route("/deposit", web::post().to(deposit))
            .route("/withdraw", web::post().to(withdraw))
            .route("/get_shares", web::post().to(get_shares))
            .route("/update_shares", web::post().to(update_shares))
            .route("/test_rpc", web::get().to(test_rpc))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
    .context("Failed to start HTTP server")
}

async fn listen_for_events(app_state: web::Data<AppState>) -> Result<()> {
    for (chain_id, contract) in &app_state.contracts {
        let contract_clone = contract.clone();
        let processed_events_clone = app_state.processed_events.clone();
        let chain_id_clone = *chain_id;

        tokio::spawn(async move {
            info!("Starting event listener for chain ID: {}", chain_id_clone);
            
            let event_stream = contract_clone.events();
            let mut stream = event_stream.stream().await.expect("Failed to create event stream");

            while let Some(event) = stream.next().await {
                match event {
                    Ok(event) => match event {
                        DepositGraphEvents::WithdrawalRequestedFilter(withdrawal) => {
                            info!("Received WithdrawalRequested event on chain {}", chain_id_clone);
                            if let Err(e) = process_withdrawal(withdrawal, &processed_events_clone, chain_id_clone).await {
                                error!("Error processing withdrawal: {:?}", e);
                            }
                        }
                        DepositGraphEvents::SharesUpdatedFilter(update) => {
                            info!("Shares updated for user {:?}: {} on chain {}", update.user, update.new_shares, chain_id_clone);
                        }
                        _ => {} // Handle other events if necessary
                    },
                    Err(e) => error!("Error in event stream for chain {}: {:?}", chain_id_clone, e),
                }
            }
        });
    }
    Ok(())
}

async fn process_withdrawal(
    withdrawal: WithdrawalRequestedFilter,
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

    // Note: The actual transfer of ETH should be handled by the contract itself.
    // This function is mainly for logging and tracking purposes.
    events.insert(event_id, true);

    Ok(())
}
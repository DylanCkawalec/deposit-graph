use crate::contracts::AppState;
use actix_web::{web, HttpResponse, Responder};
use ethers::providers::Middleware;
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct BlobUpdate {
    blob: String,
    chain_id: U256,
}

#[derive(Deserialize)]
pub struct UserAction {
    address: Address,
    chain_id: U256,
}

#[derive(Deserialize)]
pub struct Deposit {
    address: Address,
    amount: U256,
    chain_id: U256,
}

#[derive(Deserialize)]
pub struct Withdraw {
    address: Address,
    shares: U256,
    chain_id: U256,
}

#[derive(Serialize)]
pub struct ApiResponse {
    status: String,
    tx_hash: Option<String>,
    message: String,
    data: Option<serde_json::Value>,
}

pub async fn sign_up(
    user: web::Json<UserAction>,
    data: web::Data<Arc<AppState>>,
) -> impl Responder {
    let contract = match data.contracts.get(&user.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ApiResponse {
                status: "error".to_string(),
                tx_hash: None,
                message: format!("Unsupported chain ID: {}", user.chain_id),
                data: None,
            })
        }
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

pub async fn deposit(
    deposit: web::Json<Deposit>,
    data: web::Data<Arc<AppState>>,
) -> impl Responder {
    let contract = match data.contracts.get(&deposit.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ApiResponse {
                status: "error".to_string(),
                tx_hash: None,
                message: format!("Unsupported chain ID: {}", deposit.chain_id),
                data: None,
            })
        }
    };

    match contract
        .deposit()
        .from(deposit.address)
        .value(deposit.amount)
        .send()
        .await
    {
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

pub async fn withdraw(
    withdraw: web::Json<Withdraw>,
    data: web::Data<Arc<AppState>>,
) -> impl Responder {
    let contract = match data.contracts.get(&withdraw.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ApiResponse {
                status: "error".to_string(),
                tx_hash: None,
                message: format!("Unsupported chain ID: {}", withdraw.chain_id),
                data: None,
            })
        }
    };

    match contract
        .request_withdrawal(withdraw.shares)
        .from(withdraw.address)
        .send()
        .await
    {
        Ok(tx) => {
            let tx_hash = format!("{:?}", tx.tx_hash());
            HttpResponse::Ok().json(ApiResponse {
                status: "success".to_string(),
                tx_hash: Some(tx_hash),
                message: "Withdrawal request successful".to_string(),
                data: None,
            })
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse {
            status: "error".to_string(),
            tx_hash: None,
            message: format!("Error requesting withdrawal: {:?}", e),
            data: None,
        }),
    }
}

pub async fn get_shares(
    user: web::Json<UserAction>,
    data: web::Data<Arc<AppState>>,
) -> impl Responder {
    let contract = match data.contracts.get(&user.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ApiResponse {
                status: "error".to_string(),
                tx_hash: None,
                message: format!("Unsupported chain ID: {}", user.chain_id),
                data: None,
            })
        }
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

pub async fn update_shares(
    blob: web::Json<BlobUpdate>,
    data: web::Data<Arc<AppState>>,
) -> impl Responder {
    let contract = match data.contracts.get(&blob.chain_id) {
        Some(c) => c,
        None => {
            return HttpResponse::BadRequest().json(ApiResponse {
                status: "error".to_string(),
                tx_hash: None,
                message: format!("Unsupported chain ID: {}", blob.chain_id),
                data: None,
            })
        }
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

pub async fn test_rpc(data: web::Data<Arc<AppState>>) -> impl Responder {
    let contract = data.contracts.values().next().unwrap();
    let provider = contract.client();

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

use deposit_graph::{
    api, config, contracts::{self, AppState},
    events,
};
use actix_web::{web, App, HttpServer};
use std::sync::Arc;
use tracing::{info, error};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let config = config::AppConfig::from_env().expect("Failed to load configuration");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .init();

    info!("Starting DepositGraph service");

    let contracts = contracts::initialize_contracts(&config).await.expect("Failed to initialize contracts");

    let app_state = web::Data::new(Arc::new(AppState {
        contracts,
        processed_events: Arc::new(RwLock::new(HashMap::new())),
        drpc_api_key: config.drpc_api_key.clone(),
    }));

    let app_state_clone = app_state.clone();

    tokio::spawn(async move {
        if let Err(e) = events::listen_for_events(app_state_clone).await {
            error!("Error in event listener: {:?}", e);
        }
    });

    info!("Starting HTTP server on {}:{}", config.host, config.port);
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .configure(api::config)
    })
    .bind((config.host.clone(), config.port))?
    .run()
    .await
}
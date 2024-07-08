use actix_web::{web, App, HttpServer};
use deposit_graph::{
    api, config,
    contracts::{self, AppState},
    events,
};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};
use std::path::Path;

fn load_env_file(path: &str) -> std::io::Result<()> {
    let contents = fs::read_to_string(path)?;
    println!("Contents of {}:", path);
    for line in contents.lines() {
        if !line.starts_with('#') && !line.is_empty() {
            let parts: Vec<&str> = line.splitn(2, '=').collect();
            if parts.len() == 2 {
                let key = parts[0].trim();
                let value = parts[1].trim();
                println!("  {}: {}", key, if key.contains("KEY") { "[REDACTED]" } else { value });
                env::set_var(key, value);
            }
        }
    }
    Ok(())
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let env_paths = vec![".env", "../.env", "../../.env"];
    for path in env_paths {
        if Path::new(path).exists() {
            load_env_file(path)?;
            println!("Loaded .env from {}", path);
            break;
        }
    }

    let config = config::AppConfig::from_env().expect("Failed to load configuration");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_span_events(FmtSpan::CLOSE)
        .init();

    info!("Starting DepositGraph service");

    let contracts = match contracts::initialize_contracts(&config).await {
        Ok(contracts) => contracts,
        Err(e) => {
            error!("Failed to initialize contracts: {:?}", e);
            std::process::exit(1);
        }
    };

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
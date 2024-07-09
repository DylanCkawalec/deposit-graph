use actix_web::{web, App, HttpServer};
use deposit_graph::{
    api, config,
    contracts::{self, AppState},
    events,
};

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

fn print_all_env_vars() {
    for (key, value) in env::vars() {
        println!(
            "{}: {}",
            key,
            if key.contains("KEY") || key.contains("PRIVATE") {
                "[REDACTED]"
            } else {
                &value
            }
        );
    }
}

fn print_env_file_contents(path: &str) {
    match fs::read_to_string(path) {
        Ok(contents) => {
            println!("Contents of {}:", path);
            for line in contents.lines() {
                if !line.starts_with('#') && !line.is_empty() {
                    let parts: Vec<&str> = line.splitn(2, '=').collect();
                    if parts.len() == 2 {
                        println!(
                            "  {}: {}",
                            parts[0],
                            if parts[0].contains("KEY") {
                                "[REDACTED]"
                            } else {
                                parts[1]
                            }
                        );
                    }
                }
            }
        }
        Err(e) => println!("Failed to read {}: {}", path, e),
    }
}

fn load_env_file() {
    let current_dir = env::current_dir().expect("Failed to get current directory");
    let mut dir = current_dir.as_path();

    loop {
        let env_path = dir.join(".env");
        if env_path.exists() {
            println!("Found .env file at: {}", env_path.display());
            let content = fs::read_to_string(&env_path).expect("Failed to read .env file");
            for line in content.lines() {
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim();
                    if !key.starts_with('#') && !key.is_empty() {
                        env::set_var(key, value);
                        if key.contains("KEY") || key.contains("PRIVATE") {
                            println!("Set environment variable: {} = [REDACTED]", key);
                        } else {
                            println!("Set environment variable: {} = {}", key, value);
                        }
                    }
                }
            }
            return;
        }

        if let Some(parent) = dir.parent() {
            dir = parent;
        } else {
            println!("Could not find .env file in any parent directory");
            return;
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    load_env_file();

    println!("All environment variables after loading .env:");
    print_all_env_vars();

    let env_paths = vec![".env", "../.env", "../../.env"];
    for path in env_paths {
        if Path::new(path).exists() {
            print_env_file_contents(path);
            dotenv::from_path(path).ok();
            println!("Loaded .env from {}", path);
            break;
        }
    }

    println!("All environment variables after loading .env:");
    print_all_env_vars();

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

    let app_config = web::Data::new(config.clone()); // Clone config here

    let app_state_clone = app_state.clone();
    let app_config_clone = app_config.clone();

    tokio::spawn(async move {
        if let Err(e) = events::listen_for_events(app_state_clone, app_config_clone).await {
            error!("Error in event listener: {:?}", e);
        }
    });

    info!("Starting HTTP server on {}:{}", config.host, config.port);
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .app_data(app_config.clone())
            .configure(api::config)
    })
    .bind((config.host.clone(), config.port))?
    .run()
    .await
}

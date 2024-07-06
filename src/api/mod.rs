mod handlers;

use actix_web::web;

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/api")
            .route("/sign_up", web::post().to(handlers::sign_up))
            .route("/deposit", web::post().to(handlers::deposit))
            .route("/withdraw", web::post().to(handlers::withdraw))
            .route("/get_shares", web::post().to(handlers::get_shares))
            .route("/update_shares", web::post().to(handlers::update_shares))
            .route("/test_rpc", web::get().to(handlers::test_rpc))
    );
}
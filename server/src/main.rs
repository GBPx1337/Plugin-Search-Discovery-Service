mod app;
mod config;
mod dom;
mod infra;
mod present;
mod shared;

use axum::{routing::get, Router};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    tracing::info!("server binary — fill me in at step 13");
    Ok(())
}
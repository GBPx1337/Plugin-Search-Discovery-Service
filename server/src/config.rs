use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub db_max_connections: u32,
    pub db_connect_timeout: Duration,
    pub run_migrations: bool,
}

impl AppConfig {

    pub fn from_env() -> anyhow::Result<Self> {

        let _ = dotenvy::dotenv();

        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is not set"))?;

        if database_url.trim().is_empty() {
            return Err(anyhow::anyhow!("DATABASE_URL is empty"));
        }

        let server_host =
            std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let server_port = std::env::var("SERVER_PORT")
            .ok()
            .map(|s| {
                s.parse::<u16>()
                    .map_err(|e| anyhow::anyhow!("SERVER_PORT is not a valid u16: {e}"))
            })
            .transpose()?
            .unwrap_or(8080);

        let db_max_connections = std::env::var("DB_MAX_CONNECTIONS")
            .ok()
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|e| anyhow::anyhow!("DB_MAX_CONNECTIONS is not a u32: {e}"))
            })
            .transpose()?
            .unwrap_or(10);

        if db_max_connections == 0 {
            return Err(anyhow::anyhow!("DB_MAX_CONNECTIONS must be > 0"));
        }

        let db_connect_timeout = std::env::var("DB_CONNECT_TIMEOUT_SECS")
            .ok()
            .map(|s| {
                s.parse::<u64>()
                    .map_err(|e| anyhow::anyhow!("DB_CONNECT_TIMEOUT_SECS is not a u64: {e}"))
            })
            .transpose()?
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(8));

        let run_migrations = std::env::var("RUN_MIGRATIONS")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);

        Ok(Self {
            database_url,
            server_host,
            server_port,
            db_max_connections,
            db_connect_timeout,
            run_migrations,
        })
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server_host, self.server_port)
    }

    pub fn init_tracing(&self) {
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,sea_orm=warn,sqlx=warn"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .init();
    }
}
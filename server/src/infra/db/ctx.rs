use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::repo::PluginRepo;
use crate::dom::plugins::uow::PluginUoW;
use crate::infra::db::persist::plugins::repo::RepoImpl;

#[derive(Clone)]
pub struct AppCtx {
    db: DatabaseConnection,
}

impl AppCtx {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// «Свободный» репозиторий — вне транзакции.
    pub fn plugins(&self) -> RepoImpl<'_, DatabaseConnection> {
        RepoImpl::from_db(&self.db)
    }

    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }
}

#[async_trait]
impl PluginUoW for AppCtx {
    async fn transaction<T, F>(&self, f: F) -> Result<T, PluginError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
            &'a dyn PluginRepo,
        )
            -> Pin<Box<dyn Future<Output = Result<T, PluginError>> + Send + 'a>>
        + Send,
    {
        let tx = self
            .db
            .begin()
            .await
            .map_err(|e: DbErr| PluginError::Internal(e.into()))?;

        let repo: RepoImpl<'_, DatabaseTransaction> = RepoImpl::from_tx(&tx);

        match f(&repo).await {
            Ok(value) => {
                tx.commit()
                    .await
                    .map_err(|e| PluginError::Internal(e.into()))?;
                Ok(value)
            }
            Err(e) => {
                // Явный rollback не обязателен (Drop сделает), но безопаснее.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}
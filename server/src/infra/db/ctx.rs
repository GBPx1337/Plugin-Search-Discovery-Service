use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::repo::PluginRepo;
use crate::dom::plugins::uow::PluginUoW;
use crate::infra::db::persist::plugins::repo::RepoImpl;
use crate::shared::Page;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};

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

#[async_trait]
impl PluginRepo for AppCtx {
    async fn search(
        &self,
        criteria: SearchCriteria,
    ) -> Result<Page<Plugin>, PluginError> {
        self.plugins().search(criteria).await
    }

    async fn upsert(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        // На «свободном» соединении upsert НЕ БЕЗОПАСЕН из-за race condition.
        // Но этот метод не должен вызываться напрямую: app всегда вызывает
        // upsert через uow.transaction(), который даёт репозиторий с транзакцией.
        //
        // Оставляем метод на случай прямого вызова — предупредим в логе.
        tracing::warn!("AppCtx::upsert called outside of transaction — risk of race condition");
        self.plugins().upsert(plugin).await
    }

    async fn delete(&self, id: PluginId) -> Result<(), PluginError> {
        self.plugins().delete(id).await
    }
}
use async_trait::async_trait;
use sea_orm::ConnectionTrait;

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};
use crate::dom::plugins::repo::PluginRepo;
use crate::shared::Page;

/// Реализация `PluginRepo` поверх SeaORM.
///
/// Generic по `C: ConnectionTrait` — это позволяет использовать одну и ту же
/// структуру и с `DatabaseConnection`, и с `DatabaseTransaction`.
pub struct RepoImpl<'a, C: ConnectionTrait> {
    conn: &'a C,
}

impl<'a, C: ConnectionTrait> RepoImpl<'a, C> {
    /// Создаёт репозиторий поверх обычного соединения (пула).
    pub fn from_db(conn: &'a C) -> Self {
        Self { conn }
    }

    /// Создаёт репозиторий поверх транзакции.
    ///
    /// Отдельное имя — просто для читаемости на месте вызова:
    /// `RepoImpl::from_tx(&tx)` понятнее, чем `RepoImpl::from_db(&tx)`.
    pub fn from_tx(conn: &'a C) -> Self {
        Self { conn }
    }

    /// Доступ к соединению — нужен внутри методов репозитория.
    pub(crate) fn conn(&self) -> &'a C {
        self.conn
    }
}

#[async_trait]
impl<'a, C: ConnectionTrait + Send + Sync> PluginRepo for RepoImpl<'a, C> {
    async fn search(&self, _criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError> {
        Err(PluginError::Internal(anyhow::anyhow!(
            "RepoImpl::search not implemented yet"
        )))
    }

    async fn upsert(&self, _plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        Err(PluginError::Internal(anyhow::anyhow!(
            "RepoImpl::upsert not implemented yet"
        )))
    }

    async fn delete(&self, _id: PluginId) -> Result<(), PluginError> {
        Err(PluginError::Internal(anyhow::anyhow!(
            "RepoImpl::delete not implemented yet"
        )))
    }
}
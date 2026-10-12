use async_trait::async_trait;
use uuid::Uuid;

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};
use crate::shared::Page;

#[async_trait]
pub trait PluginRepo: Send + Sync {
    /// Поиск плагинов по критериям.
    async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError>;

    /// Создаёт или обновляет запись в поисковом индексе.
    async fn upsert(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError>;

    /// Удаляет запись из индекса.
    async fn delete(&self, id: PluginId) -> Result<(), PluginError>;
}
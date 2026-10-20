use std::sync::Arc;

use crate::app::services::plugin_search_service::PluginSearchService;
use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};
use crate::shared::Page;

#[async_trait::async_trait]
pub trait PluginSearch: Send + Sync + 'static {
    async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError>;
    async fn index_plugin(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError>;
    async fn delete_plugin(&self, id: PluginId) -> Result<(), PluginError>;
}

#[async_trait::async_trait]
impl<C> PluginSearch for PluginSearchService<C>
where
    C: crate::dom::plugins::repo::PluginRepo
    + crate::dom::plugins::uow::PluginUoW
    + Send
    + Sync
    + 'static,
{
    async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError> {
        PluginSearchService::search(self, criteria).await
    }

    async fn index_plugin(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        PluginSearchService::index_plugin(self, plugin).await
    }

    async fn delete_plugin(&self, id: PluginId) -> Result<(), PluginError> {
        PluginSearchService::delete_plugin(self, id).await
    }
}

#[derive(Clone)]
pub struct AppState {
    pub plugin_search: Arc<dyn PluginSearch>,
}

impl AppState {
    pub fn new<C>(plugin_search: PluginSearchService<C>) -> Self
    where
        C: crate::dom::plugins::repo::PluginRepo
        + crate::dom::plugins::uow::PluginUoW
        + Send
        + Sync
        + 'static,
    {
        Self {
            plugin_search: Arc::new(plugin_search),
        }
    }
}
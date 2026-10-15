use std::sync::Arc;

use crate::app::services::plugin_search_service::PluginSearchService;
use crate::dom::plugins::model::{Plugin, SearchCriteria};
use crate::dom::plugins::errors::PluginError;
use crate::shared::Page;

/// Трейт-фасад сервиса поиска, чтобы AppState не был generic.
#[async_trait::async_trait]
pub trait PluginSearch: Send + Sync + 'static {
    async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError>;
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
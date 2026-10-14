use crate::app::services::plugin_search_service::PluginSearchService;
use crate::dom::plugins::repo::PluginRepo;
use crate::dom::plugins::uow::PluginUoW;

/// Состояние приложения для HTTP-слоя.
///
/// Generic по `C: PluginRepo + PluginUoW` — `present` не знает, SeaORM там
/// или in-memory реализация.
pub struct AppState<C: PluginRepo + PluginUoW> {
    pub plugin_search: PluginSearchService<C>,
}

impl<C: PluginRepo + PluginUoW> Clone for AppState<C> {
    fn clone(&self) -> Self {
        Self {
            plugin_search: self.plugin_search.clone(),
        }
    }
}

impl<C: PluginRepo + PluginUoW> AppState<C> {
    pub fn new(plugin_search: PluginSearchService<C>) -> Self {
        Self { plugin_search }
    }
}
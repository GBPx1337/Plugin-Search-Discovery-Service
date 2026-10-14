use std::sync::Arc;

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};
use crate::dom::plugins::repo::PluginRepo;
use crate::dom::plugins::uow::PluginUoW;
use crate::shared::Page;

/// Прикладной сервис поиска и индексирования плагинов.
///
/// Параметризован `U: PluginUoW` — конкретный тип Unit of Work
/// (`AppCtx` в проде, `InMemoryUoW` в тестах).
/// Репозиторий берётся как `Arc<dyn PluginRepo>` — он object-safe.
pub struct PluginSearchService<U: PluginUoW> {
    repo: Arc<dyn PluginRepo>,
    uow: Arc<U>,
}

// Ручной `Clone` — `Arc<U>` клонируется независимо от `U: Clone`.
impl<U: PluginUoW> Clone for PluginSearchService<U> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            uow: Arc::clone(&self.uow),
        }
    }
}

impl<U: PluginUoW> PluginSearchService<U> {
    pub fn new(repo: Arc<dyn PluginRepo>, uow: Arc<U>) -> Self {
        Self { repo, uow }
    }

    /// Поиск плагинов по критериям.
    ///
    /// Транзакция не открывается: одиночный SELECT атомарен.
    pub async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError> {
        let criteria = criteria.normalized();
        criteria.validate()?;
        self.repo.search(criteria).await
    }

    /// Создать или обновить запись в поисковом индексе.
    ///
    /// Выполняется в транзакции: `upsert` — check-then-act,
    /// атомарность обязательна.
    pub async fn index_plugin(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        let uow = Arc::clone(&self.uow);
        uow.transaction(move |repo| {
            let plugin = plugin.clone();
            Box::pin(async move { repo.upsert(plugin).await })
        })
            .await
    }

    /// Удалить запись из поискового индекса. Идемпотентно.
    pub async fn delete_plugin(&self, id: PluginId) -> Result<(), PluginError> {
        self.repo.delete(id).await
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use super::*;
    use crate::app::services::test_mocks::{InMemoryPluginRepo, InMemoryUoW};
    use crate::dom::plugins::model::{PluginStatus, SortOrder};

    fn make_plugin(
        id: Uuid,
        name: &str,
        status: PluginStatus,
    ) -> Plugin {
        Plugin {
            id: PluginId::new(id),
            name: name.into(),
            description: "desc".into(),
            category: "productivity".into(),
            tags: vec!["pdf".into()],
            price: 1000,
            rating: Decimal::new(45, 1),
            downloads: 100,
            status,
            updated_at: Utc::now().fixed_offset(),
        }
    }

    /// Возвращаем конкретный тип сервиса, а не `dyn`.
    /// `U = InMemoryUoW` — реальная in-memory реализация Unit of Work.
    fn make_service() -> PluginSearchService<InMemoryUoW> {
        let repo: Arc<dyn PluginRepo> = Arc::new(InMemoryPluginRepo::default());
        let uow = Arc::new(InMemoryUoW::new(Arc::clone(&repo)));
        PluginSearchService::new(repo, uow)
    }

    #[tokio::test]
    async fn search_validates_criteria() {
        let svc = make_service();
        let criteria = SearchCriteria {
            q: None,
            category: None,
            tags: vec![],
            min_price: None,
            max_price: None,
            min_rating: None,
            sort: SortOrder::Relevance,
            page: 0, // <-- невалидно
            limit: 20,
        };

        let res = svc.search(criteria).await;
        assert!(matches!(res, Err(PluginError::Validation(_))));
    }

    #[tokio::test]
    async fn index_creates_then_noop() {
        let svc = make_service();
        let id = Uuid::new_v4();
        let p = make_plugin(id, "A", PluginStatus::Published);

        let created = svc.index_plugin(p.clone()).await.unwrap();
        assert_eq!(created, UpsertOutcome::Created);

        let noop = svc.index_plugin(p.clone()).await.unwrap();
        assert_eq!(noop, UpsertOutcome::NoOp);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let svc = make_service();
        let id = Uuid::new_v4();
        let p = make_plugin(id, "A", PluginStatus::Published);

        svc.index_plugin(p).await.unwrap();
        svc.delete_plugin(PluginId::new(id)).await.unwrap();
        // Второй раз — не ошибка
        svc.delete_plugin(PluginId::new(id)).await.unwrap();
    }

    #[tokio::test]
    async fn search_returns_only_published() {
        let svc = make_service();
        svc.index_plugin(make_plugin(Uuid::new_v4(), "Pub", PluginStatus::Published))
            .await
            .unwrap();
        svc.index_plugin(make_plugin(Uuid::new_v4(), "Draft", PluginStatus::Draft))
            .await
            .unwrap();

        let criteria = SearchCriteria {
            q: None,
            category: None,
            tags: vec![],
            min_price: None,
            max_price: None,
            min_rating: None,
            sort: SortOrder::Relevance,
            page: 1,
            limit: 20,
        };
        let page = svc.search(criteria).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "Pub");
    }
}
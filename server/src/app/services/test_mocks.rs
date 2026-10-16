//! Mock-реализации доменных портов для тестов `app`-слоя.
//! Не используются в runtime.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{
    Plugin, PluginId, SearchCriteria, SortOrder, UpsertOutcome,
};
use crate::dom::plugins::repo::PluginRepo;
use crate::shared::Page;

#[derive(Default)]
pub struct InMemoryPluginRepo {
    inner: Mutex<HashMap<PluginId, Plugin>>,
}

#[async_trait]
impl PluginRepo for InMemoryPluginRepo {
    async fn search(
        &self,
        criteria: SearchCriteria,
    ) -> Result<Page<Plugin>, PluginError> {
        let criteria = criteria.normalized();
        let store = self.inner.lock().unwrap();

        let mut filtered: Vec<Plugin> = store
            .values()
            .filter(|p| p.status == crate::dom::plugins::model::PluginStatus::Published)
            .filter(|p| match &criteria.q {
                Some(q) => {
                    let q = q.to_lowercase();
                    p.name.to_lowercase().contains(&q) || p.description.to_lowercase().contains(&q)
                }
                None => true,
            })
            .filter(|p| match &criteria.category {
                Some(c) => &p.category == c,
                None => true,
            })
            .filter(|p| {
                criteria.tags.iter().all(|t| p.tags.contains(t))
            })
            .filter(|p| criteria.min_price.map_or(true, |m| p.price >= m))
            .filter(|p| criteria.max_price.map_or(true, |m| p.price <= m))
            .filter(|p| criteria.min_rating.map_or(true, |m| p.rating >= m))
            .cloned()
            .collect();

        // Сортировка. Тот же контракт, что и у RepoImpl.
        match criteria.sort {
            SortOrder::Relevance => {
                if let Some(q) = &criteria.q {
                    let q = q.to_lowercase();
                    filtered.sort_by(|a, b| {
                        let a_score = if a.name.to_lowercase().contains(&q) { 0 } else { 1 };
                        let b_score = if b.name.to_lowercase().contains(&q) { 0 } else { 1 };
                        a_score.cmp(&b_score).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
                    });
                } else {
                    filtered.sort_by_key(|p| p.id.into_inner());
                }
            }
            SortOrder::PriceAsc => filtered.sort_by(|a, b| {
                a.price.cmp(&b.price).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
            }),
            SortOrder::PriceDesc => filtered.sort_by(|a, b| {
                b.price.cmp(&a.price).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
            }),
            SortOrder::RatingDesc => filtered.sort_by(|a, b| {
                b.rating.cmp(&a.rating).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
            }),
            SortOrder::DownloadsDesc => filtered.sort_by(|a, b| {
                b.downloads.cmp(&a.downloads).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
            }),
            SortOrder::Newest => filtered.sort_by(|a, b| {
                b.updated_at.cmp(&a.updated_at).then_with(|| a.id.into_inner().cmp(&b.id.into_inner()))
            }),
        }

        let total = filtered.len() as u64;
        let offset = (criteria.page as usize - 1) * criteria.limit as usize;
        let items: Vec<Plugin> = filtered
            .into_iter()
            .skip(offset)
            .take(criteria.limit as usize)
            .collect();

        Ok(Page::new(items, total, criteria.page, criteria.limit))
    }

    async fn upsert(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        let mut store = self.inner.lock().unwrap();

        match store.get(&plugin.id) {
            None => {
                store.insert(plugin.id, plugin);
                Ok(UpsertOutcome::Created)
            }
            Some(existing) => {
                use std::cmp::Ordering;
                match plugin.updated_at.cmp(&existing.updated_at) {
                    Ordering::Less => Ok(UpsertOutcome::Stale),
                    Ordering::Equal => {
                        if content_eq(existing, &plugin) {
                            Ok(UpsertOutcome::NoOp)
                        } else {
                            Ok(UpsertOutcome::Conflict)
                        }
                    }
                    Ordering::Greater => {
                        store.insert(plugin.id, plugin);
                        Ok(UpsertOutcome::Updated)
                    }
                }
            }
        }
    }

    async fn delete(&self, id: PluginId) -> Result<(), PluginError> {
        self.inner.lock().unwrap().remove(&id);
        Ok(())
    }
}

fn content_eq(a: &Plugin, b: &Plugin) -> bool {
    a.id == b.id
        && a.name == b.name
        && a.description == b.description
        && a.category == b.category
        && a.tags == b.tags
        && a.price == b.price
        && a.rating == b.rating
        && a.downloads == b.downloads
        && a.status == b.status
}

// Заглушка UoW: выполняет замыкание, не открывая реальной транзакции.
pub struct InMemoryUoW {
    repo: std::sync::Arc<dyn PluginRepo>,
}

impl InMemoryUoW {
    pub fn new(repo: std::sync::Arc<dyn PluginRepo>) -> Self {
        Self { repo }
    }
}

#[async_trait::async_trait]
impl crate::dom::plugins::uow::PluginUoW for InMemoryUoW {
    async fn transaction<T, F>(&self, f: F) -> Result<T, PluginError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
            &'a dyn PluginRepo,
        )
            -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<T, PluginError>> + Send + 'a>,
            >
        + Send,
    {
        // В памяти транзакции не нужны — репозиторий и так под Mutex.
        f(self.repo.as_ref()).await
    }
}
use async_trait::async_trait;
use sea_orm::{ConnectionTrait, EntityTrait};

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, UpsertOutcome};
use crate::dom::plugins::repo::PluginRepo;
use crate::infra::db::entities::plugin as plugin_entity;
use crate::infra::db::persist::plugins::mapping;
use crate::shared::Page;

pub struct RepoImpl<'a, C: ConnectionTrait> {
    conn: &'a C,
}

impl<'a, C: ConnectionTrait> RepoImpl<'a, C> {
    pub fn from_db(conn: &'a C) -> Self {
        Self { conn }
    }

    pub fn from_tx(conn: &'a C) -> Self {
        Self { conn }
    }

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

    async fn upsert(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        // 1. Ищем существующую запись
        let existing = plugin_entity::Entity::find_by_id(plugin.id.into_inner())
            .one(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;

        match existing {
            None => {
                // 2a. Записи нет → INSERT
                let active = mapping::to_active_model(&plugin);
                plugin_entity::Entity::insert(active)
                    .exec(self.conn())
                    .await
                    .map_err(|e| PluginError::Internal(e.into()))?;
                Ok(UpsertOutcome::Created)
            }
            Some(current) => {
                use std::cmp::Ordering;

                match plugin.updated_at.cmp(&current.updated_at) {
                    Ordering::Less => {
                        // 2b. Пришло старее — Stale, ничего не делаем
                        Ok(UpsertOutcome::Stale)
                    }
                    Ordering::Equal => {
                        // 2c. Та же метка — сравниваем содержимое
                        let current_dom: Plugin = current.into();
                        if content_eq(&current_dom, &plugin) {
                            Ok(UpsertOutcome::NoOp)
                        } else {
                            Ok(UpsertOutcome::Conflict)
                        }
                    }
                    Ordering::Greater => {
                        // 2d. Пришло свежее — UPDATE
                        let active = mapping::to_active_model(&plugin);
                        plugin_entity::Entity::update(active)
                            .exec(self.conn())
                            .await
                            .map_err(|e| PluginError::Internal(e.into()))?;
                        Ok(UpsertOutcome::Updated)
                    }
                }
            }
        }
    }

    async fn delete(&self, id: PluginId) -> Result<(), PluginError> {
        plugin_entity::Entity::delete_by_id(id.into_inner())
            .exec(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;
        // Намеренно игнорируем rows_affected — повторный delete безвреден.
        Ok(())
    }
}

/// Сравнивает содержимое плагинов, **исключая** `updated_at`.
/// Используется для различения NoOp и Conflict.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone, Utc};
    use rust_decimal::Decimal;
    use sea_orm::{Database, TransactionTrait};
    use uuid::Uuid;

    use crate::dom::plugins::model::PluginStatus;
    use crate::dom::plugins::uow::PluginUoW;
    use crate::infra::db::ctx::AppCtx;

    async fn setup() -> AppCtx {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let db = Database::connect(&url).await.expect("connect");
        // ВАЖНО: тесты должны работать на чистой БД. Очистим таблицу.
        use sea_orm::ConnectionTrait;
        db.execute_unprepared("TRUNCATE plugins").await.unwrap();
        AppCtx::new(db)
    }

    fn sample(id: Uuid, name: &str, updated_at_min: u32) -> Plugin {
        let ts = Utc
            .with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
            .unwrap()
            + chrono::Duration::minutes(updated_at_min as i64);
        Plugin {
            id: PluginId::new(id),
            name: name.into(),
            description: "desc".into(),
            category: "productivity".into(),
            tags: vec!["pdf".into()],
            price: 1000,
            rating: Decimal::new(47, 1),
            downloads: 10,
            status: PluginStatus::Published,
            updated_at: ts.fixed_offset(),
        }
    }

    #[tokio::test]
    async fn upsert_creates_when_missing() {
        let ctx = setup().await;
        let p = sample(Uuid::new_v4(), "A", 0);

        let outcome = ctx
            .transaction(|repo| {
                let p = p.clone();
                Box::pin(async move { repo.upsert(p).await })
            })
            .await
            .unwrap();

        assert_eq!(outcome, UpsertOutcome::Created);
    }

    #[tokio::test]
    async fn upsert_updates_when_newer() {
        let ctx = setup().await;
        let id = Uuid::new_v4();

        // Сначала создаём
        ctx.transaction(|repo| {
            let p = sample(id, "A", 0);
            Box::pin(async move { repo.upsert(p).await })
        })
            .await
            .unwrap();

        // Обновляем свежим
        let outcome = ctx
            .transaction(|repo| {
                let p = sample(id, "A updated", 5);
                Box::pin(async move { repo.upsert(p).await })
            })
            .await
            .unwrap();

        assert_eq!(outcome, UpsertOutcome::Updated);
    }

    #[tokio::test]
    async fn upsert_is_noop_on_identical_repeat() {
        let ctx = setup().await;
        let id = Uuid::new_v4();
        let p = sample(id, "A", 10);

        ctx.transaction(|repo| {
            let p = p.clone();
            Box::pin(async move { repo.upsert(p).await })
        })
            .await
            .unwrap();

        let outcome = ctx
            .transaction(|repo| {
                let p = p.clone();
                Box::pin(async move { repo.upsert(p).await })
            })
            .await
            .unwrap();

        assert_eq!(outcome, UpsertOutcome::NoOp);
    }

    #[tokio::test]
    async fn upsert_rejects_stale() {
        let ctx = setup().await;
        let id = Uuid::new_v4();

        ctx.transaction(|repo| {
            let p = sample(id, "A", 10);
            Box::pin(async move { repo.upsert(p).await })
        })
            .await
            .unwrap();

        // Приходит с более старым updated_at
        let outcome = ctx
            .transaction(|repo| {
                let p = sample(id, "A older", 5);
                Box::pin(async move { repo.upsert(p).await })
            })
            .await
            .unwrap();

        assert_eq!(outcome, UpsertOutcome::Stale);
    }

    #[tokio::test]
    async fn upsert_conflicts_on_same_timestamp_different_content() {
        let ctx = setup().await;
        let id = Uuid::new_v4();

        ctx.transaction(|repo| {
            let p = sample(id, "A", 10);
            Box::pin(async move { repo.upsert(p).await })
        })
            .await
            .unwrap();

        let outcome = ctx
            .transaction(|repo| {
                let p = sample(id, "Different", 10);
                Box::pin(async move { repo.upsert(p).await })
            })
            .await
            .unwrap();

        assert_eq!(outcome, UpsertOutcome::Conflict);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let ctx = setup().await;
        let id = Uuid::new_v4();

        ctx.transaction(|repo| {
            let p = sample(id, "A", 0);
            Box::pin(async move { repo.upsert(p).await })
        })
            .await
            .unwrap();

        let r1 = ctx
            .transaction(|repo| {
                Box::pin(async move { repo.delete(PluginId::new(id)).await })
            })
            .await;
        assert!(r1.is_ok());

        // Повторный delete — не ошибка
        let r2 = ctx
            .transaction(|repo| {
                Box::pin(async move { repo.delete(PluginId::new(id)).await })
            })
            .await;
        assert!(r2.is_ok());
    }
}
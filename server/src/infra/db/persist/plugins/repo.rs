use async_trait::async_trait;
use sea_orm::sea_query::{ArrayType, Condition, Expr, Order, SimpleExpr, Value};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, SearchCriteria, SortOrder, UpsertOutcome};
use crate::dom::plugins::repo::PluginRepo;
use crate::infra::db::entities::plugin::{
    Entity as PluginEntity,
    Column
};
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
    async fn search(&self, criteria: SearchCriteria) -> Result<Page<Plugin>, PluginError> {
        let criteria = criteria.normalized();

        let (items, total) = tokio::try_join!(
            self.search_items(&criteria),
            self.search_total(&criteria),
        )?;

        Ok(Page::new(items, total, criteria.page, criteria.limit))
    }

    async fn upsert(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
        let existing = PluginEntity::find_by_id(plugin.id.into_inner())
            .one(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;

        match existing {
            None => {
                let active = mapping::to_active_model(&plugin);
                PluginEntity::insert(active)
                    .exec(self.conn())
                    .await
                    .map_err(|e| PluginError::Internal(e.into()))?;
                Ok(UpsertOutcome::Created)
            }
            Some(current) => {
                //TOMBSTONE: запись удалена
                if let Some(deleted_at) = current.deleted_at {
                    if plugin.updated_at <= deleted_at {
                        return Ok(UpsertOutcome::DeletedStale);
                    }
                    let active = mapping::to_active_model(&plugin);
                    PluginEntity::update(active)
                        .exec(self.conn())
                        .await
                        .map_err(|e| PluginError::Internal(e.into()))?;
                    return Ok(UpsertOutcome::Updated);
                }

                //TOMBSTONE нет: обычная логика
                use std::cmp::Ordering;
                match plugin.updated_at.cmp(&current.updated_at) {
                    Ordering::Less => Ok(UpsertOutcome::Stale),
                    Ordering::Equal => {
                        let current_dom: Plugin = current.into();
                        if content_eq(&current_dom, &plugin) {
                            Ok(UpsertOutcome::NoOp)
                        } else {
                            Ok(UpsertOutcome::Conflict)
                        }
                    }
                    Ordering::Greater => {
                        let active = mapping::to_active_model(&plugin);
                        PluginEntity::update(active)
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
        use sea_orm::sea_query::Expr;

        let now = chrono::Utc::now().fixed_offset();

        PluginEntity::update_many()
            .col_expr(Column::DeletedAt, Expr::value(now))
            .filter(Column::Id.eq(id.into_inner()))
            .filter(Column::DeletedAt.is_null())   //повторный delete не меняет deleted_at
            .exec(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;

        Ok(())
    }
}

// ---------------- helpers ----------------

impl<'a, C: ConnectionTrait + Send + Sync> RepoImpl<'a, C> {
    async fn search_items(&self, criteria: &SearchCriteria) -> Result<Vec<Plugin>, PluginError> {
        let where_cond = build_where(criteria);
        let mut query = PluginEntity::find().filter(where_cond);

        for (expr, order) in build_order(criteria) {
            query = query.order_by(expr, order);
        }

        let offset = (criteria.page as u64 - 1) * criteria.limit as u64;
        query = query.limit(criteria.limit as u64).offset(offset);

        let rows = query
            .all(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;

        Ok(rows.into_iter().map(Plugin::from).collect())
    }

    async fn search_total(&self, criteria: &SearchCriteria) -> Result<u64, PluginError> {
        let where_cond = build_where(criteria);

        let total = PluginEntity::find()
            .filter(where_cond)
            .count(self.conn())
            .await
            .map_err(|e| PluginError::Internal(e.into()))?;

        Ok(total)
    }
}

fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn build_where(criteria: &SearchCriteria) -> Condition {
    let mut cond = Condition::all();

    cond = cond.add(Column::Status.eq("published"));
    cond = cond.add(Column::DeletedAt.is_null());

    if let Some(q) = criteria.q.as_deref() {
        let pattern = format!("%{}%", escape_like(q));
        let name_match = Expr::cust_with_values("name ILIKE $1", [pattern.clone()]);
        let desc_match = Expr::cust_with_values("description ILIKE $1", [pattern]);
        cond = cond.add(Condition::any().add(name_match).add(desc_match));
    }

    if let Some(cat) = criteria.category.as_deref() {
        cond = cond.add(Column::Category.eq(cat));
    }

    if !criteria.tags.is_empty() {
        let inner: Vec<Value> = criteria
            .tags
            .iter()
            .map(|t| Value::from(t.clone()))
            .collect();
        let array_value = Value::Array(ArrayType::String, Some(Box::new(inner)));
        cond = cond.add(Expr::cust_with_values("tags @> $1", [array_value]));
    }

    if let Some(min) = criteria.min_price {
        cond = cond.add(Column::Price.gte(min));
    }
    if let Some(max) = criteria.max_price {
        cond = cond.add(Column::Price.lte(max));
    }
    if let Some(min_r) = criteria.min_rating {
        cond = cond.add(Column::Rating.gte(min_r));
    }

    cond
}

fn build_order(criteria: &SearchCriteria) -> Vec<(SimpleExpr, Order)> {
    let tie = || (Expr::col(Column::Id), Order::Asc);

    match criteria.sort {
        SortOrder::Relevance => {
            if let Some(q) = criteria.q.as_deref() {
                let pattern = format!("%{}%", escape_like(q));
                let relevance: SimpleExpr = Expr::cust_with_values(
                    "CASE WHEN name ILIKE $1 THEN 0 ELSE 1 END",
                    [pattern],
                );
                vec![(relevance, Order::Asc), tie()]
            } else {
                vec![tie()]
            }
        }
        SortOrder::PriceAsc => vec![(Expr::col(Column::Price), Order::Asc), tie()],
        SortOrder::PriceDesc => vec![(Expr::col(Column::Price), Order::Desc), tie()],
        SortOrder::RatingDesc => vec![(Expr::col(Column::Rating), Order::Desc), tie()],
        SortOrder::DownloadsDesc => vec![(Expr::col(Column::Downloads), Order::Desc), tie()],
        SortOrder::Newest => vec![(Expr::col(Column::UpdatedAt), Order::Desc), tie()],
    }
}

/// Сравнивает содержимое плагинов, исключая `updated_at`.
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
    use chrono::Utc;
    use rust_decimal::Decimal;
    use sea_orm::{ConnectionTrait, Database};
    use uuid::Uuid;

    use crate::dom::plugins::model::{PluginStatus, SearchCriteria, SortOrder};
    use crate::infra::db::ctx::AppCtx;
    use crate::dom::plugins::uow::PluginUoW;

    fn sample(
        id: Uuid,
        name: &str,
        description: &str,
        category: &str,
        tags: &[&str],
        price: i32,
        rating: &str,
        downloads: i64,
        status: PluginStatus,
    ) -> Plugin {
        Plugin {
            id: PluginId::new(id),
            name: name.into(),
            description: description.into(),
            category: category.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            price,
            rating: Decimal::from_str_exact(rating).unwrap(),
            downloads,
            status,
            updated_at: Utc::now().fixed_offset(),
        }
    }

    fn criteria() -> SearchCriteria {
        SearchCriteria {
            q: None,
            category: None,
            tags: vec![],
            min_price: None,
            max_price: None,
            min_rating: None,
            sort: SortOrder::Relevance,
            page: 1,
            limit: 20,
        }
    }

    async fn setup_with(plugins: Vec<Plugin>) -> AppCtx {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let db = Database::connect(&url).await.expect("connect");
        db.execute_unprepared("TRUNCATE plugins").await.unwrap();
        let ctx = AppCtx::new(db);

        for p in plugins {
            ctx.transaction(|repo| {
                let p = p.clone();
                Box::pin(async move { repo.upsert(p).await.map(|_| ()) })
            })
                .await
                .unwrap();
        }

        ctx
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let ctx = setup_with(vec![sample(
            Uuid::new_v4(), "PDF Export", "Export to PDF", "productivity",
            &["pdf"], 1000, "4.5", 100, PluginStatus::Published,
        )]).await;

        let mut c = criteria();
        c.q = Some("pdf".into()); // нижний регистр

        let page = ctx.plugins().search(c).await.unwrap();
        assert_eq!(page.total, 1);
    }

    #[tokio::test]
    async fn search_partial_name_match() {
        let ctx = setup_with(vec![
            sample(Uuid::new_v4(), "Excel Export Tool", "d", "productivity", &[], 0, "4.0", 0, PluginStatus::Published),
            sample(Uuid::new_v4(), "PDF Viewer", "d", "productivity", &[], 0, "4.0", 0, PluginStatus::Published),
        ]).await;

        let mut c = criteria();
        c.q = Some("Export".into());
        let page = ctx.plugins().search(c).await.unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "Excel Export Tool");
    }

    #[tokio::test]
    async fn search_combined_filters() {
        let ctx = setup_with(vec![
            // проходит все фильтры
            sample(Uuid::new_v4(), "A", "d", "productivity", &[], 1000, "4.5", 0, PluginStatus::Published),
            // не проходит по цене
            sample(Uuid::new_v4(), "B", "d", "productivity", &[], 5000, "4.5", 0, PluginStatus::Published),
            // не проходит по рейтингу
            sample(Uuid::new_v4(), "C", "d", "productivity", &[], 1000, "3.5", 0, PluginStatus::Published),
            // не проходит по категории
            sample(Uuid::new_v4(), "D", "d", "design", &[], 1000, "4.5", 0, PluginStatus::Published),
        ]).await;

        let mut c = criteria();
        c.category = Some("productivity".into());
        c.min_price = Some(500);
        c.max_price = Some(2000);
        c.min_rating = Some(Decimal::from_str_exact("4.0").unwrap());

        let page = ctx.plugins().search(c).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "A");
    }

    #[tokio::test]
    async fn search_excludes_non_published() {
        let ctx = setup_with(vec![
            sample(Uuid::new_v4(), "Pub", "d", "c", &[], 0, "4", 0, PluginStatus::Published),
            sample(Uuid::new_v4(), "Draft", "d", "c", &[], 0, "4", 0, PluginStatus::Draft),
            sample(Uuid::new_v4(), "Arch", "d", "c", &[], 0, "4", 0, PluginStatus::Archived),
        ]).await;

        let page = ctx.plugins().search(criteria()).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "Pub");
    }

    #[tokio::test]
    async fn search_sort_deterministic_on_equal_rating() {
        let ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
        let mut plugins = Vec::new();
        for id in &ids {
            plugins.push(sample(*id, "X", "d", "c", &[], 0, "4.5", 0, PluginStatus::Published));
        }
        let ctx = setup_with(plugins).await;

        let mut c = criteria();
        c.sort = SortOrder::RatingDesc;
        let page = ctx.plugins().search(c).await.unwrap();

        // Все с одинаковым рейтингом — должны идти по id ASC
        let mut expected = ids.clone();
        expected.sort();
        let got: Vec<Uuid> = page.items.iter().map(|p| p.id.into_inner()).collect();
        assert_eq!(got, expected);
    }

    #[tokio::test]
    async fn search_total_ignores_limit() {
        let mut plugins = Vec::new();
        for i in 0..50 {
            plugins.push(sample(Uuid::new_v4(), &format!("P{i}"), "d", "c", &[], 0, "4", 0, PluginStatus::Published));
        }
        let ctx = setup_with(plugins).await;

        let mut c = criteria();
        c.limit = 10;
        let page = ctx.plugins().search(c).await.unwrap();

        assert_eq!(page.items.len(), 10);
        assert_eq!(page.total, 50);
        assert_eq!(page.total_pages(), 5);
    }

    #[tokio::test]
    async fn search_tags_and_semantics() {
        let ctx = setup_with(vec![
            sample(Uuid::new_v4(), "Both", "d", "c", &["pdf", "export"], 0, "4", 0, PluginStatus::Published),
            sample(Uuid::new_v4(), "OnlyPdf", "d", "c", &["pdf"], 0, "4", 0, PluginStatus::Published),
            sample(Uuid::new_v4(), "OnlyExport", "d", "c", &["export"], 0, "4", 0, PluginStatus::Published),
        ]).await;

        let mut c = criteria();
        c.tags = vec!["pdf".into(), "export".into()];
        let page = ctx.plugins().search(c).await.unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].name, "Both");
    }

    #[tokio::test]
    async fn search_relevance_prioritizes_name() {
        let ctx = setup_with(vec![
            // совпадение только в description
            sample(Uuid::new_v4(), "Zeta", "some export stuff", "c", &[], 0, "4", 0, PluginStatus::Published),
            // совпадение в name
            sample(Uuid::new_v4(), "Export Pro", "nope", "c", &[], 0, "4", 0, PluginStatus::Published),
        ]).await;

        let mut c = criteria();
        c.q = Some("export".into());
        c.sort = SortOrder::Relevance;
        let page = ctx.plugins().search(c).await.unwrap();

        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].name, "Export Pro"); // name-совпадение первым
    }

    #[tokio::test]
    async fn search_pagination_second_page() {
        let mut plugins = Vec::new();
        for i in 0..25 {
            plugins.push(sample(
                Uuid::new_v4(),
                &format!("P{i:03}"),
                "d", "c", &[], 0, "4", i as i64, PluginStatus::Published,
            ));
        }
        let ctx = setup_with(plugins).await;

        let mut c = criteria();
        c.sort = SortOrder::DownloadsDesc;
        c.limit = 10;
        c.page = 3;
        let page = ctx.plugins().search(c).await.unwrap();

        assert_eq!(page.items.len(), 5); // остаток на 3-й странице
        assert_eq!(page.total, 25);
        // Последние по downloads — самые маленькие
        assert!(page.items.iter().all(|p| p.downloads < 10));
    }
}
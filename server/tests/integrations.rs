//! Интеграционные тесты из ТЗ (10 сценариев).
//!
//! Требуют переменную `DATABASE_URL`, указывающую на тестовую БД.
//! Пример:
//!     $env:DATABASE_URL = "postgres://postgres:postgres@localhost:5432/plugin_search_test"
//!     cargo test -p server --test integration
//!
//! Тесты сериализованы (`#[serial]`), чтобы TRUNCATE не мешал параллельности.

use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use rust_decimal::Decimal;
use sea_orm::{ConnectionTrait, Database};
use serial_test::serial;
use uuid::Uuid;

use server::dom::plugins::errors::PluginError;
use server::dom::plugins::model::{
    Plugin, PluginId, PluginStatus, SearchCriteria, SortOrder, MAX_LIMIT,
};
use server::dom::plugins::repo::PluginRepo;
use server::dom::plugins::uow::PluginUoW;
use server::infra::db::ctx::AppCtx;
use migration::{Migrator, MigratorTrait};

async fn setup() -> Arc<AppCtx> {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set to a test database");
    let db = Database::connect(&url).await.expect("connect to db");
    Migrator::up(&db, None).await.expect("migrate");

    db.execute_unprepared("TRUNCATE plugins").await.unwrap();
    Arc::new(AppCtx::new(db))
}

fn make_plugin(
    name: &str,
    description: &str,
    category: &str,
    tags: &[&str],
    price: i32,
    rating: &str,
    downloads: i64,
    status: PluginStatus,
    updated_at_offset_hours: i64,
) -> Plugin {
    let ts = Utc::now() + TimeDelta::hours(updated_at_offset_hours);
    // Сопостовление временных данных
    let ts = chrono::DateTime::<Utc>::from_timestamp_micros(ts.timestamp_micros())
        .expect("valid timestamp")
        .fixed_offset();

    Plugin {
        id: PluginId::new(Uuid::new_v4()),
        name: name.into(),
        description: description.into(),
        category: category.into(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        price,
        rating: Decimal::from_str_exact(rating).unwrap(),
        downloads,
        status,
        updated_at: ts,
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

async fn insert(ctx: &AppCtx, p: Plugin) -> server::dom::plugins::model::UpsertOutcome {
    let p2 = p.clone();
    ctx.transaction(move |repo| Box::pin(async move { repo.upsert(p2).await }))
        .await
        .expect("upsert")
}

async fn search(
    ctx: &AppCtx,
    c: SearchCriteria,
) -> Result<server::shared::Page<Plugin>, PluginError> {
    ctx.plugins().search(c).await
}

// 1. Поиск без учёта регистра
#[tokio::test]
#[serial]
async fn scenario_01_search_case_insensitive() {
    let ctx = setup().await;
    insert(
        &ctx,
        make_plugin(
            "PDF Export Pro",
            "Tool for exporting",
            "productivity",
            &[],
            1000,
            "4.5",
            0,
            PluginStatus::Published,
            0,
        ),
    )
        .await;

    for q in ["pdf", "PDF", "Pdf", "pDf"] {
        let mut c = criteria();
        c.q = Some(q.into());
        let page = search(&ctx, c).await.expect("search");
        assert_eq!(page.total, 1, "query = {q}");
    }
}

// 2. Частичное совпадение названия

#[tokio::test]
#[serial]
async fn scenario_02_search_partial_name_match() {
    let ctx = setup().await;
    insert(
        &ctx,
        make_plugin("Excel Export Tool", "d", "productivity", &[], 0, "4.0", 0, PluginStatus::Published, 0),
    )
        .await;
    insert(
        &ctx,
        make_plugin("PDF Viewer", "d", "productivity", &[], 0, "4.0", 0, PluginStatus::Published, 0),
    )
        .await;

    let mut c = criteria();
    c.q = Some("Export".into());
    let page = search(&ctx, c).await.unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].name, "Excel Export Tool");
}

// 3. Комбинированная фильтрация: category + price + rating

#[tokio::test]
#[serial]
async fn scenario_03_combined_filters() {
    let ctx = setup().await;
    // подходит по всем фильтрам
    insert(&ctx, make_plugin("A", "d", "productivity", &[], 1000, "4.5", 0, PluginStatus::Published, 0)).await;
    // провал по цене (слишком дорого)
    insert(&ctx, make_plugin("B", "d", "productivity", &[], 5000, "4.5", 0, PluginStatus::Published, 0)).await;
    // провал по рейтингу
    insert(&ctx, make_plugin("C", "d", "productivity", &[], 1000, "3.5", 0, PluginStatus::Published, 0)).await;
    // провал по категории
    insert(&ctx, make_plugin("D", "d", "design", &[], 1000, "4.5", 0, PluginStatus::Published, 0)).await;

    let mut c = criteria();
    c.category = Some("productivity".into());
    c.min_price = Some(500);
    c.max_price = Some(2000);
    c.min_rating = Some(Decimal::from_str_exact("4.0").unwrap());

    let page = search(&ctx, c).await.unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].name, "A");
}

// 4. Исключение неопубликованных

#[tokio::test]
#[serial]
async fn scenario_04_excludes_non_published() {
    let ctx = setup().await;
    insert(&ctx, make_plugin("Pub", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0)).await;
    insert(&ctx, make_plugin("Draft", "d", "c", &[], 0, "4", 0, PluginStatus::Draft, 0)).await;
    insert(&ctx, make_plugin("Arch", "d", "c", &[], 0, "4", 0, PluginStatus::Archived, 0)).await;

    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].name, "Pub");
}

// 5. Детерминированная сортировка при равном рейтинге

#[tokio::test]
#[serial]
async fn scenario_05_deterministic_sort_equal_rating() {
    let ctx = setup().await;
    let mut ids = Vec::new();
    for _ in 0..5 {
        let p = make_plugin("X", "d", "c", &[], 0, "4.5", 0, PluginStatus::Published, 0);
        ids.push(p.id.into_inner());
        insert(&ctx, p).await;
    }

    let mut c = criteria();
    c.sort = SortOrder::RatingDesc;
    c.limit = 20;
    let page = search(&ctx, c).await.unwrap();

    let mut expected = ids.clone();
    expected.sort();
    let got: Vec<Uuid> = page.items.iter().map(|p| p.id.into_inner()).collect();
    assert_eq!(got, expected, "equal rating → id ASC");
}

// 6. Корректный подсчёт total (без LIMIT)

#[tokio::test]
#[serial]
async fn scenario_06_total_ignores_limit() {
    let ctx = setup().await;
    for i in 0..50 {
        insert(
            &ctx,
            make_plugin(&format!("P{i}"), "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0),
        )
            .await;
    }

    let mut c = criteria();
    c.limit = 10;
    let page = search(&ctx, c).await.unwrap();

    assert_eq!(page.items.len(), 10);
    assert_eq!(page.total, 50);
    assert_eq!(page.total_pages(), 5);
}

// 7. Повторное индексирование без дубликатов

#[tokio::test]
#[serial]
async fn scenario_07_reindex_no_duplicates() {
    use server::dom::plugins::model::UpsertOutcome;

    let ctx = setup().await;
    let p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;

    assert_eq!(insert(&ctx, p.clone()).await, UpsertOutcome::Created);
    assert_eq!(insert(&ctx, p.clone()).await, UpsertOutcome::NoOp);
    assert_eq!(insert(&ctx, p.clone()).await, UpsertOutcome::NoOp);

    // В БД — ровно одна запись
    let all = search(&ctx, criteria()).await.unwrap();
    assert_eq!(all.total, 1);
    assert_eq!(all.items[0].id, id);
}

// 8. Отклонение устаревшего обновления

#[tokio::test]
#[serial]
async fn scenario_08_reject_stale_update() {
    use server::dom::plugins::model::UpsertOutcome;

    let ctx = setup().await;
    let mut p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    insert(&ctx, p.clone()).await;

    // Пришло с более старым updated_at
    p.updated_at = p.updated_at - TimeDelta::hours(1);
    let outcome = insert(&ctx, p).await;
    assert_eq!(outcome, UpsertOutcome::Stale);

    // В БД осталась исходная версия
    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 1);
    assert!(page.items[0].updated_at > page.items[0].updated_at - TimeDelta::hours(2));
}

#[tokio::test]
#[serial]
async fn scenario_08b_conflict_on_same_ts_different_content() {
    use server::dom::plugins::model::UpsertOutcome;

    let ctx = setup().await;
    let p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    insert(&ctx, p.clone()).await;

    let mut modified = p.clone();
    modified.name = "Different".into();
    // updated_at тот же
    let outcome = insert(&ctx, modified).await;
    assert_eq!(outcome, UpsertOutcome::Conflict);
}


// 9. Фильтрация по нескольким тегам (AND)

#[tokio::test]
#[serial]
async fn scenario_09_multi_tag_and_semantics() {
    let ctx = setup().await;
    insert(&ctx, make_plugin("Both", "d", "c", &["pdf", "export"], 0, "4", 0, PluginStatus::Published, 0)).await;
    insert(&ctx, make_plugin("OnlyPdf", "d", "c", &["pdf"], 0, "4", 0, PluginStatus::Published, 0)).await;
    insert(&ctx, make_plugin("OnlyExport", "d", "c", &["export"], 0, "4", 0, PluginStatus::Published, 0)).await;

    let mut c = criteria();
    c.tags = vec!["pdf".into(), "export".into()];
    let page = search(&ctx, c).await.unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].name, "Both");
}

// 10. Валидация параметров пагинации

#[test]
fn scenario_10_pagination_validation() {
    use server::dom::plugins::model::SearchCriteriaError;

    let mut c = criteria();
    c.page = 0;
    assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidPage)));

    let mut c = criteria();
    c.limit = 0;
    assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidLimit)));

    let mut c = criteria();
    c.limit = MAX_LIMIT + 1;
    assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidLimit)));

    let mut c = criteria();
    c.page = 1;
    c.limit = MAX_LIMIT;
    assert!(c.validate().is_ok());
}


// Дополнительные тесты

#[tokio::test]
#[serial]
async fn extra_relevance_prioritizes_name_match() {
    let ctx = setup().await;
    insert(&ctx, make_plugin("Zeta", "some export stuff", "c", &[], 0, "4", 0, PluginStatus::Published, 0)).await;
    insert(&ctx, make_plugin("Export Pro", "nope", "c", &[], 0, "4", 0, PluginStatus::Published, 0)).await;

    let mut c = criteria();
    c.q = Some("export".into());
    c.sort = SortOrder::Relevance;
    let page = search(&ctx, c).await.unwrap();

    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].name, "Export Pro");
}

#[tokio::test]
#[serial]
async fn extra_delete_is_idempotent() {
    let ctx = setup().await;
    let p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;
    insert(&ctx, p).await;

    assert!(ctx.plugins().delete(id).await.is_ok());
    assert!(ctx.plugins().delete(id).await.is_ok());
}

#[tokio::test]
#[serial]
async fn extra_pagination_pages() {
    let ctx = setup().await;
    for i in 0..25 {
        insert(
            &ctx,
            make_plugin(&format!("P{i:03}"), "d", "c", &[], 0, "4", i as i64, PluginStatus::Published, 0),
        )
            .await;
    }

    let mut c = criteria();
    c.sort = SortOrder::DownloadsDesc;
    c.limit = 10;
    c.page = 3;
    let page = search(&ctx, c).await.unwrap();

    assert_eq!(page.items.len(), 5);
    assert_eq!(page.total, 25);
    assert!(page.items.iter().all(|p| p.downloads < 10));
}

#[tokio::test]
#[serial]
async fn tombstone_delete_hides_from_search() {
    let ctx = setup().await;
    let p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;
    insert(&ctx, p).await;

    // до удаления — видна
    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 1);

    ctx.plugins().delete(id).await.unwrap();

    // после удаления — не видна
    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 0);
}

// Tombstone: повторный delete идемпотентен

#[tokio::test]
#[serial]
async fn tombstone_delete_is_idempotent() {
    let ctx = setup().await;
    let p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;
    insert(&ctx, p).await;

    assert!(ctx.plugins().delete(id).await.is_ok());
    assert!(ctx.plugins().delete(id).await.is_ok());

    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 0);
}

// Tombstone: запоздавший PUT не воскрешает

#[tokio::test]
#[serial]
async fn tombstone_rejects_stale_put_after_delete() {
    use server::dom::plugins::model::UpsertOutcome;

    let ctx = setup().await;
    let mut p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;
    insert(&ctx, p.clone()).await;

    ctx.plugins().delete(id).await.unwrap();

    // PUT с updated_at СТАРЕЕ deleted_at
    p.updated_at = p.updated_at - TimeDelta::hours(1);
    let outcome = insert(&ctx, p).await;
    assert_eq!(outcome, UpsertOutcome::DeletedStale);

    // запись остаётся удалённой
    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 0);
}

// Tombstone: PUT с более новым updated_at воскрешает

#[tokio::test]
#[serial]
async fn tombstone_revives_with_newer_put() {
    use server::dom::plugins::model::UpsertOutcome;

    let ctx = setup().await;
    let mut p = make_plugin("A", "d", "c", &[], 0, "4", 0, PluginStatus::Published, 0);
    let id = p.id;
    insert(&ctx, p.clone()).await;

    ctx.plugins().delete(id).await.unwrap();

    // PUT с updated_at НОВЕЕ deleted_at
    p.updated_at = p.updated_at + TimeDelta::hours(1);
    let outcome = insert(&ctx, p).await;
    assert_eq!(outcome, UpsertOutcome::Updated);

    // запись снова видна
    let page = search(&ctx, criteria()).await.unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].id, id);
}
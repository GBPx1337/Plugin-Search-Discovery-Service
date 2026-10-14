use server::dom::plugins::model::{SearchCriteria, SortOrder};
use server::dom::plugins::repo::PluginRepo;
use server::infra::db::ctx::AppCtx;
use sea_orm::Database;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")?;
    let db = Database::connect(&url).await?;
    let ctx = AppCtx::new(db);

    let criteria = SearchCriteria {
        q: Some("export".into()),
        category: Some("productivity".into()),
        tags: vec![],
        min_price: None,
        max_price: None,
        min_rating: Some(rust_decimal::Decimal::new(40, 1)),
        sort: SortOrder::DownloadsDesc,
        page: 1,
        limit: 5,
    };

    let started = std::time::Instant::now();
    let page = ctx.plugins().search(criteria).await?;
    let elapsed = started.elapsed();

    println!("total: {}", page.total);
    println!("page: {}, limit: {}, total_pages: {}", page.page, page.limit, page.total_pages());
    println!("elapsed: {:?}", elapsed);
    for p in &page.items {
        println!("  {} | {} | {} | {}", p.name, p.category, p.rating, p.downloads);
    }
    Ok(())
}
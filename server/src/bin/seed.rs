//! Генератор тестовых данных для таблицы `plugins`.
//!
//! Запуск:
//!     cargo run -p server --bin seed -- --count 100000 --seed 42
//!
//! Требует `DATABASE_URL` в окружении (или `.env`).

use chrono::Utc;
use clap::Parser;
use fake::faker::lorem::en::{Sentence, Words};
use fake::Fake;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rust_decimal::Decimal;
use sea_orm::{Database, DatabaseConnection, EntityTrait};
use uuid::Uuid;

use server::dom::plugins::model::PluginStatus;
use server::infra::db::entities::plugin::{ActiveModel, Entity as PluginEntity};
use server::infra::db::persist::plugins::mapping;
use server::dom::plugins::model::{Plugin, PluginId};

#[derive(Parser, Debug)]
#[command(name = "seed", about = "Fill plugins table with test data")]
struct Args {
    /// Сколько записей создать.
    #[arg(long, default_value_t = 100_000)]
    count: usize,

    /// Seed для воспроизводимости. Если не задан — берётся из системы.
    #[arg(long)]
    seed: Option<u64>,

    /// Размер батча для INSERT.
    #[arg(long, default_value_t = 1000)]
    batch_size: usize,

    /// Очистить таблицу перед наполнением.
    #[arg(long, default_value_t = false)]
    truncate: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL is not set"))?;

    tracing::info!(
        count = args.count,
        batch_size = args.batch_size,
        truncate = args.truncate,
        seed = ?args.seed,
        "starting seed"
    );

    let db = Database::connect(&database_url).await?;

    if args.truncate {
        tracing::warn!("truncating plugins table");
        use sea_orm::ConnectionTrait;
        db.execute_unprepared("TRUNCATE plugins").await?;
    }

    let mut rng: StdRng = match args.seed {
        Some(s) => StdRng::seed_from_u64(s),
        None => StdRng::from_entropy(),
    };

    let started = std::time::Instant::now();
    let mut inserted = 0usize;

    while inserted < args.count {
        let remaining = args.count - inserted;
        let chunk = remaining.min(args.batch_size);

        let batch: Vec<ActiveModel> = (0..chunk)
            .map(|_| {
                let plugin = generate_plugin(&mut rng);
                mapping::to_active_model(&plugin)
            })
            .collect();

        insert_batch(&db, batch).await?;
        inserted += chunk;

        if inserted % 10_000 == 0 || inserted == args.count {
            let elapsed = started.elapsed().as_secs_f64();
            let rate = inserted as f64 / elapsed.max(0.001);
            tracing::info!(
                inserted,
                total = args.count,
                rate_per_sec = format!("{:.0}", rate),
                elapsed_sec = format!("{:.1}", elapsed),
                "progress"
            );
        }
    }

    tracing::info!(
        total = args.count,
        elapsed_sec = format!("{:.1}", started.elapsed().as_secs_f64()),
        "seed complete"
    );

    Ok(())
}

async fn insert_batch(db: &DatabaseConnection, batch: Vec<ActiveModel>) -> anyhow::Result<()> {
    PluginEntity::insert_many(batch).exec(db).await?;
    Ok(())
}

// ---------------- Генерация одной записи ----------------

const CATEGORIES: &[&str] = &[
    "productivity",
    "analytics",
    "design",
    "developer-tools",
    "communication",
    "security",
];

const TAG_POOL: &[&str] = &[
    "pdf", "export", "import", "csv", "json", "api", "sync", "cloud",
    "backup", "chart", "report", "ai", "automation", "webhook", "auth",
    "encryption", "realtime", "search", "notifications", "dashboard",
];

/// Слова, которые часто встречаются в названиях/описаниях.
/// Нужны, чтобы запрос `q=export` находил осмысленные записи.
const KEYWORDS: &[&str] = &[
    "Export", "Import", "Sync", "Backup", "Chart", "Report",
    "Analytics", "Dashboard", "Automation", "Workflow", "Manager",
    "Connector", "Integration", "Assistant", "Monitor", "Tracker",
];

fn generate_plugin(rng: &mut StdRng) -> Plugin {
    let id = Uuid::new_v4();
    let name = generate_name(rng);
    let description = generate_description(rng);
    let category = CATEGORIES[rng.gen_range(0..CATEGORIES.len())].to_string();
    let tags = generate_tags(rng);
    let price = generate_price(rng);
    let rating = generate_rating(rng);
    let downloads = generate_downloads(rng);
    let status = generate_status(rng);
    let updated_at = generate_updated_at(rng);

    Plugin {
        id: PluginId::new(id),
        name,
        description,
        category,
        tags,
        price,
        rating,
        downloads,
        status,
        updated_at,
    }
}

fn generate_name(rng: &mut StdRng) -> String {
    let kw = KEYWORDS[rng.gen_range(0..KEYWORDS.len())];
    let words: Vec<String> = Words(1..3).fake_with_rng(rng);
    let suffix = words.join(" ");
    format!("{kw} {suffix}")
        .split_whitespace()
        .map(capitalize)
        .collect::<Vec<_>>()
        .join(" ")
}

fn generate_description(rng: &mut StdRng) -> String {
    // 1–2 предложения
    let s: String = Sentence(8..20).fake_with_rng(rng);
    if rng.gen_bool(0.4) {
        let s2: String = Sentence(6..15).fake_with_rng(rng);
        format!("{s} {s2}")
    } else {
        s
    }
}

fn generate_tags(rng: &mut StdRng) -> Vec<String> {
    let n = rng.gen_range(1..=4);
    let mut chosen = Vec::with_capacity(n);
    while chosen.len() < n {
        let t = TAG_POOL[rng.gen_range(0..TAG_POOL.len())];
        if !chosen.iter().any(|c: &String| c == t) {
            chosen.push(t.to_string());
        }
    }
    chosen
}

fn generate_price(rng: &mut StdRng) -> i32 {
    if rng.gen_bool(0.10) {
        0
    } else {
        let exp = rng.gen_range(8..=14); // 2^8 = 256, 2^14 = 16384
        let base = 1i32 << exp;
        let jitter = rng.gen_range(0..(base / 2).max(1));
        (base + jitter).min(500_000)
    }
}

fn generate_rating(rng: &mut StdRng) -> Decimal {
    // Смещаем к высоким: 60% в [4, 5], 30% в [3, 4), 10% в [0, 3)
    let f: f64 = if rng.gen_bool(0.60) {
        rng.gen_range(4.0..=5.0)
    } else if rng.gen_bool(0.75) {
        rng.gen_range(3.0..4.0)
    } else {
        rng.gen_range(0.0..3.0)
    };
    // Округляем до 2 знаков
    let scaled = (f * 100.0).round() as i64;
    Decimal::new(scaled, 2)
}

fn generate_downloads(rng: &mut StdRng) -> i64 {
    // Pareto-ish: r^3, домножаем на 500k
    let r: f64 = rng.gen_range(0.0..1.0);
    let value = (r.powi(3) * 500_000.0) as i64;
    value.max(0)
}

fn generate_status(rng: &mut StdRng) -> PluginStatus {
    let x = rng.gen_range(0..100);
    match x {
        0..70 => PluginStatus::Published,
        70..90 => PluginStatus::Draft,
        _ => PluginStatus::Archived,
    }
}

fn generate_updated_at(rng: &mut StdRng) -> chrono::DateTime<chrono::FixedOffset> {
    use chrono::TimeDelta;

    let r: f64 = rng.gen_range(0.0..1.0);
    let max_days = 730.0;
    let days_ago = (r.powi(2) * max_days) as i64;
    let secs_jitter = rng.gen_range(0..86_400) as i64;

    let ts = Utc::now()
        - TimeDelta::days(days_ago)
        - TimeDelta::seconds(secs_jitter);

    ts.into() // DateTime<Utc> → DateTime<FixedOffset>
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}
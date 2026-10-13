use sea_orm::{DatabaseConnection, DatabaseTransaction, DbErr, TransactionTrait};

use crate::infra::db::persist::plugins::repo::RepoImpl;

/// «Контейнер» инфраструктурных зависимостей.
///
/// Держит пул соединений и предоставляет доступ к репозиториям.
/// Не содержит бизнес-логики.
#[derive(Clone)]
pub struct AppCtx {
    db: DatabaseConnection,
}

impl AppCtx {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Доступ к «свободному» репозиторию — без транзакции.
    ///
    /// Подходит для одиночных операций, где атомарность обеспечена
    /// одним SQL-запросом (например, простой поиск или DELETE).
    pub fn plugins(&self) -> RepoImpl<'_, DatabaseConnection> {
        RepoImpl::from_db(&self.db)
    }

    /// Выполняет замыкание внутри одной транзакции.
    ///
    /// Внутри замыкания приходит `Repos`, привязанный к транзакции.
    /// Если замыкание возвращает `Err`, транзакция откатывается.
    /// Если `Ok`, коммитится.
    pub async fn transaction<F, T, Fut, E>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(Repos<'_>) -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: From<DbErr>,
    {
        let tx = self.db.begin().await?;
        let repos = Repos {
            plugins: RepoImpl::from_tx(&tx),
        };
        match f(repos).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(e) => {
                // Явный rollback не обязателен — Drop сделает его.
                // Но лучше явно, чтобы не полагаться на неявное поведение.
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    /// Даёт прямой доступ к `DatabaseConnection` — только для composition root
    /// и инфраструктурных нужд. Бизнес-код не должен этим пользоваться.
    pub fn connection(&self) -> &DatabaseConnection {
        &self.db
    }
}

/// Набор репозиториев внутри транзакции.
///
/// Если появятся новые сущности — добавь сюда новые поля.
pub struct Repos<'a> {
    pub plugins: RepoImpl<'a, DatabaseTransaction>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    /// Загружает .env один раз на весь процесс тестов.
    /// `dotenvy::dotenv()` безопасно вызывать повторно — он не перезапишет
    /// уже установленные переменные.
    fn load_env() {
        let _ = dotenvy::dotenv();
    }

    async fn test_ctx() -> AppCtx {
        load_env();
        let url = std::env::var("DATABASE_URL")
            .expect("DATABASE_URL must be set (см. .env.example)");
        let db = Database::connect(&url).await.expect("cannot connect to DB");
        AppCtx::new(db)
    }

    #[tokio::test]
    async fn transaction_commits_on_ok() {
        let ctx = test_ctx().await;

        let result: Result<i32, DbErr> = ctx
            .transaction(|_repos| async move { Ok(1) })
            .await;

        assert_eq!(result.unwrap(), 1);
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_err() {
        let ctx = test_ctx().await;

        let result: Result<(), DbErr> = ctx
            .transaction(|_repos| async move {
                Err(DbErr::Custom("intentional".into()))
            })
            .await;

        assert!(result.is_err());
    }
}
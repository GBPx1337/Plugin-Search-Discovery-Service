use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;

use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::repo::PluginRepo;

/// Абстракция Unit of Work для операций над плагинами.
///
/// Позволяет `app` выполнять несколько операций атомарно, не зная,
/// какое хранилище под капотом (Postgres, in-memory для тестов).
#[async_trait]
pub trait PluginUoW: Send + Sync {
    /// Выполняет замыкание в транзакции.
    ///
    /// Замыкание получает трейт-объект репозитория, привязанный к транзакции.
    /// Если замыкание вернёт `Err`, транзакция откатится.
    async fn transaction<T, F>(&self, f: F) -> Result<T, PluginError>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(
            &'a dyn PluginRepo,
        )
            -> Pin<Box<dyn Future<Output = Result<T, PluginError>> + Send + 'a>>
        + Send;
}
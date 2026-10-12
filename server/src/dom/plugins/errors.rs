use uuid::Uuid;

use crate::dom::plugins::model::SearchCriteriaError;

/// Ошибка домена — то, что `app` и `infra` возвращают наверх.
/// Не является HTTP-ошибкой: маппинг в статусы делает `present`.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin {0} not found")]
    NotFound(Uuid),

    #[error("stale update for plugin {id}: incoming {incoming}, existing {existing}")]
    Stale {
        id: Uuid,
        incoming: chrono::DateTime<chrono::FixedOffset>,
        existing: chrono::DateTime<chrono::FixedOffset>,
    },

    #[error("conflict for plugin {0}: same timestamp, different content")]
    Conflict(Uuid),

    #[error("validation error: {0}")]
    Validation(String),
    
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl From<SearchCriteriaError> for PluginError {
    fn from(e: SearchCriteriaError) -> Self {
        Self::Validation(e.to_string())
    }
}
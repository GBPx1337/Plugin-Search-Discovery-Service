use rust_decimal::Decimal;
use uuid::Uuid;

/// Доменный идентификатор плагина.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginId(pub Uuid);

impl PluginId {
    pub fn new(id: Uuid) -> Self {
        Self(id)
    }

    pub fn into_inner(self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for PluginId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Статус плагина в маркетплейсе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginStatus {
    Draft,
    Published,
    Archived,
}

impl PluginStatus {
    /// Только опубликованные плагины видны в поиске.
    pub fn is_searchable(&self) -> bool {
        matches!(self, Self::Published)
    }
}

/// Доменная модель плагина.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub id: PluginId,
    pub name: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub price: i32,
    pub rating: Decimal,
    pub downloads: i64,
    pub status: PluginStatus,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}
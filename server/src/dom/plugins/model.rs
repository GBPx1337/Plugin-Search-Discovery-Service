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

/// Способ сортировки результатов поиска.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Relevance,
    PriceAsc,
    PriceDesc,
    RatingDesc,
    DownloadsDesc,
    Newest,
}

impl SortOrder {
    /// Значение по умолчанию — релевантность.
    pub fn default_order() -> Self {
        Self::Relevance
    }
}

/// Критерии поиска плагинов.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchCriteria {
    /// Подстрока для поиска по `name` и `description`. Регистронезависимо.
    pub q: Option<String>,
    /// Точное совпадение по категории.
    pub category: Option<String>,
    /// Должны присутствовать ВСЕ указанные теги (AND).
    pub tags: Vec<String>,
    /// Нижняя граница цены включительно.
    pub min_price: Option<i32>,
    /// Верхняя граница цены включительно.
    pub max_price: Option<i32>,
    /// Нижняя граница рейтинга включительно.
    pub min_rating: Option<Decimal>,
    pub sort: SortOrder,
    pub page: u32,
    pub limit: u32,
}

/// Максимально допустимый limit из ТЗ.
pub const MAX_LIMIT: u32 = 100;

/// Ошибка валидации критериев.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchCriteriaError {
    #[error("page must be >= 1")]
    InvalidPage,
    #[error("limit must be between 1 and {MAX_LIMIT}")]
    InvalidLimit,
    #[error("min_price must be <= max_price")]
    InvalidPriceRange,
    #[error("min_rating must be between 0 and 5")]
    InvalidRating,
}

impl SearchCriteria {
    /// Бизнес-валидация. Вызывается на входе в `app`-сервис.
    pub fn validate(&self) -> Result<(), SearchCriteriaError> {
        if self.page < 1 {
            return Err(SearchCriteriaError::InvalidPage);
        }
        if self.limit < 1 || self.limit > MAX_LIMIT {
            return Err(SearchCriteriaError::InvalidLimit);
        }
        if let (Some(min), Some(max)) = (self.min_price, self.max_price) {
            if min > max {
                return Err(SearchCriteriaError::InvalidPriceRange);
            }
        }
        if let Some(r) = self.min_rating {
            if r < Decimal::ZERO || r > Decimal::new(5, 0) {
                return Err(SearchCriteriaError::InvalidRating);
            }
        }
        Ok(())
    }

    pub fn normalized(mut self) -> Self {
        self.q = self
            .q
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self
    }
}

/// Результат операции upsert - важен для правильного HTTP-статуса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Created,
    Updated,
    NoOp,
    Stale,
    Conflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_criteria() -> SearchCriteria {
        SearchCriteria {
            q: None,
            category: None,
            tags: vec![],
            min_price: None,
            max_price: None,
            min_rating: None,
            sort: SortOrder::default_order(),
            page: 1,
            limit: 20,
        }
    }

    #[test]
    fn valid_criteria_passes() {
        assert!(base_criteria().validate().is_ok());
    }

    #[test]
    fn page_zero_fails() {
        let mut c = base_criteria();
        c.page = 0;
        assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidPage)));
    }

    #[test]
    fn limit_too_large_fails() {
        let mut c = base_criteria();
        c.limit = 101;
        assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidLimit)));
    }

    #[test]
    fn price_range_inverted_fails() {
        let mut c = base_criteria();
        c.min_price = Some(100);
        c.max_price = Some(50);
        assert!(matches!(c.validate(), Err(SearchCriteriaError::InvalidPriceRange)));
    }

    #[test]
    fn normalization_trims_and_drops_empty() {
        let c = base_criteria();
        let mut c = SearchCriteria { q: Some("  export  ".into()), ..c };
        assert_eq!(c.clone().normalized().q, Some("export".into()));

        let c2 = SearchCriteria { q: Some("   ".into()), ..c };
        assert_eq!(c2.normalized().q, None);
    }
}
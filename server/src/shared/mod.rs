use serde::{Deserialize, Serialize};

/// Результат пагинированного запроса.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: u64, page: u32, limit: u32) -> Self {
        Self {
            items,
            total,
            page,
            limit,
        }
    }

    /// Количество страниц при текущем лимите. `ceil(total / limit)`.
    pub fn total_pages(&self) -> u32 {
        if self.limit == 0 {
            return 0;
        }
        let pages = (self.total + self.limit as u64 - 1) / self.limit as u64;
        pages.min(u32::MAX as u64) as u32
    }
}
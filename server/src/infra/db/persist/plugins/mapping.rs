use crate::dom::plugins::model::{Plugin, PluginId, PluginStatus as DomStatus};
use crate::infra::db::entities::plugin::{Model as PluginModel, PluginStatus as DbStatus};
use rust_decimal::Decimal;
use uuid::Uuid;
impl From<PluginModel> for Plugin {
    fn from(m: PluginModel) -> Self {
        Self {
            id: PluginId::new(m.id),
            name: m.name,
            description: m.description,
            category: m.category,
            tags: m.tags,
            price: m.price,
            rating: m.rating,
            downloads: m.downloads,
            status: m.status.into(),
            updated_at: m.updated_at,
        }
    }
}

impl From<DbStatus> for DomStatus {
    fn from(s: DbStatus) -> Self {
        match s {
            DbStatus::Draft => DomStatus::Draft,
            DbStatus::Published => DomStatus::Published,
            DbStatus::Archived => DomStatus::Archived,
        }
    }
}

impl From<DomStatus> for DbStatus {
    fn from(s: DomStatus) -> Self {
        match s {
            DomStatus::Draft => DbStatus::Draft,
            DomStatus::Published => DbStatus::Published,
            DomStatus::Archived => DbStatus::Archived,
        }
    }
}

/// Собирает ActiveModel для INSERT/UPDATE.
pub fn to_active_model(p: &Plugin) -> crate::infra::db::entities::plugin::ActiveModel {
    use sea_orm::Set;
    use crate::infra::db::entities::plugin::ActiveModel;

    ActiveModel {
        id: Set(p.id.into_inner()),
        name: Set(p.name.clone()),
        description: Set(p.description.clone()),
        category: Set(p.category.clone()),
        tags: Set(p.tags.clone()),
        price: Set(p.price),
        rating: Set(p.rating),
        downloads: Set(p.downloads),
        status: Set(p.status.into()),
        updated_at: Set(p.updated_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn roundtrip_status() {
        for db in [DbStatus::Draft, DbStatus::Published, DbStatus::Archived] {
            let dom: DomStatus = db.clone().into();
            let back: DbStatus = dom.into();
            assert_eq!(db, back);
        }
    }

    #[test]
    fn model_to_domain() {
        let now = Utc::now().fixed_offset();
        let m = PluginModel {
            id: Uuid::new_v4(),
            name: "PDF Export".into(),
            description: "Export to PDF".into(),
            category: "productivity".into(),
            tags: vec!["pdf".into(), "export".into()],
            price: 149900,
            rating: Decimal::new(47, 1), // 4.7
            downloads: 12500,
            status: DbStatus::Published,
            updated_at: now,
        };
        let p: Plugin = m.clone().into();
        assert_eq!(p.name, "PDF Export");
        assert_eq!(p.status, DomStatus::Published);
        assert_eq!(p.tags.len(), 2);
    }
}
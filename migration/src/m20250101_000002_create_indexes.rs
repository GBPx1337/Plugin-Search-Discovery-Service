use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // GIN + pg_trgm для подстрочного поиска по name (регистронезависимый)
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_name_trgm_idx \
             ON plugins USING gin (name gin_trgm_ops)",
        )
            .await?;

        // GIN + pg_trgm для description
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_description_trgm_idx \
             ON plugins USING gin (description gin_trgm_ops)",
        )
            .await?;

        // B-tree для фильтра по категории
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_category_idx \
             ON plugins (category)",
        )
            .await?;

        // B-tree для фильтра и сортировки по цене
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_price_idx \
             ON plugins (price)",
        )
            .await?;

        // B-tree для сортировки по рейтингу
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_rating_idx \
             ON plugins (rating)",
        )
            .await?;

        // B-tree для сортировки по скачиваниям
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_downloads_idx \
             ON plugins (downloads)",
        )
            .await?;

        // B-tree для сортировки по дате обновления
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_updated_at_idx \
             ON plugins (updated_at)",
        )
            .await?;

        // GIN для AND-поиска по тегам: tags @> ARRAY['a', 'b']
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_tags_gin_idx \
             ON plugins USING gin (tags)",
        )
            .await?;

        // Частичный индекс: большинство запросов смотрит только published.
        // Явный каст 'published'::plugin_status — совпадает с типом колонки.
        // updated_at DESC, id — под сортировку newest с tie-breaker по id.
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS plugins_published_idx \
             ON plugins (updated_at DESC, id) \
             WHERE status = 'published'::plugin_status",
        )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        for idx in [
            "plugins_name_trgm_idx",
            "plugins_description_trgm_idx",
            "plugins_category_idx",
            "plugins_price_idx",
            "plugins_rating_idx",
            "plugins_downloads_idx",
            "plugins_updated_at_idx",
            "plugins_tags_gin_idx",
            "plugins_published_idx",
        ] {
            conn.execute_unprepared(&format!("DROP INDEX IF EXISTS {idx}"))
                .await?;
        }
        Ok(())
    }
}
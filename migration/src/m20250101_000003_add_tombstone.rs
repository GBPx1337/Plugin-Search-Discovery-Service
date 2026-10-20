use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1. Колонка deleted_at: NULL = живая запись, NOT NULL = tombstone.
        manager
            .alter_table(
                Table::alter()
                    .table(Plugins::Table)
                    .add_column(
                        ColumnDef::new(Plugins::DeletedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // 2. Пересоздаём partial index под новое условие.
        //    Tombstones не попадают в индекс — поиск их даже не видит.
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS plugins_published_idx;
                 CREATE INDEX plugins_published_idx ON plugins (updated_at DESC, id)
                 WHERE deleted_at IS NULL AND status = 'published'::plugin_status;",
            )
            .await?;

        // 3. Индекс для фоновой очистки старых tombstones.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS plugins_deleted_at_idx
                 ON plugins (deleted_at)
                 WHERE deleted_at IS NOT NULL;",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS plugins_deleted_at_idx;
                 DROP INDEX IF EXISTS plugins_published_idx;
                 CREATE INDEX plugins_published_idx ON plugins (updated_at DESC, id)
                 WHERE status = 'published'::plugin_status;",
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Plugins::Table)
                    .drop_column(Plugins::DeletedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Plugins {
    Table,
    DeletedAt,
}
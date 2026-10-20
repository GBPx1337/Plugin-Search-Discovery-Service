use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        
        conn.execute_unprepared("CREATE EXTENSION IF NOT EXISTS pg_trgm")
            .await?;

        conn.execute_unprepared(
            "DO $$ BEGIN \
                CREATE TYPE plugin_status AS ENUM ('draft', 'published', 'archived'); \
             EXCEPTION \
                WHEN duplicate_object THEN null; \
             END $$;",
        )
            .await?;

        // 3. Таблица
        manager
            .create_table(
                Table::create()
                    .table(Plugins::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Plugins::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Plugins::Name).text().not_null())
                    .col(ColumnDef::new(Plugins::Description).text().not_null())
                    .col(ColumnDef::new(Plugins::Category).text().not_null())
                    .col(
                        ColumnDef::new(Plugins::Tags)
                            .array(ColumnType::Text)
                            .not_null()
                            .default(Expr::cust("'{}'")),
                    )
                    .col(ColumnDef::new(Plugins::Price).integer().not_null())
                    .col(
                        ColumnDef::new(Plugins::Rating)
                            .decimal_len(3, 2)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Plugins::Downloads).big_integer().not_null())
                    .col(
                        ColumnDef::new(Plugins::Status)
                            .custom(Alias::new("plugin_status"))  // <-- ключевое
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Plugins::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Plugins::Table).to_owned())
            .await?;

        // Тип удаляем ПОСЛЕ таблицы — иначе FK-зависимость
        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS plugin_status")
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Plugins {
    Table,
    Id,
    Name,
    Description,
    Category,
    Tags,
    Price,
    Rating,
    Downloads,
    Status,
    UpdatedAt,
}
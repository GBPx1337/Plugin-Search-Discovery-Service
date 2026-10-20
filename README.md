# Plugin Search & Discovery Service

Backend-сервис поиска, фильтрации и сортировки плагинов для маркетплейса.
Хранит поисковую проекцию плагинов, обновляемую из Plugin Service через внутренний API.

## Содержание

- [Стек](#стек)
- [Быстрый старт](#быстрый-старт)
- [1. Архитектура](#1-архитектура)
  - [Ключевые контракты](#ключевые-контракты)
  - [Unit of Work](#unit-of-work)
  - [Проверка границ](#проверка-границ)
- [2. Индексы PostgreSQL](#2-индексы-postgresql)
- [3. Стратегия поиска](#3-стратегия-поиска)
  - [WHERE (build_where)](#where-build_where)
  - [ORDER BY (build_order)](#order-by-build_order)
  - [LIMIT/OFFSET](#limitoffset)
- [EXPLAIN (ANALYZE, BUFFERS)](#explain-analyze-buffers)
  - [Query 1: q + downloads_desc](#query-1-q--downloads_desc)
  - [Query 2: q + relevance](#query-2-q--relevance)
  - [Query 3: category + price + rating_desc](#query-3-category--price--rating_desc)
  - [Query 4: tags AND](#query-4-tags-and)
  - [Query 5: count](#query-5-count)
  - [Общие выводы](#общие-выводы)
- [4. Ограничения OFFSET-пагинации](#4-ограничения-offset-пагинации)
- [5. Особенности работы с SeaORM](#5-особенности-работы-с-seaorm)
- [6. Подход к масштабированию каталога до 10 миллионов записей](#6-подход-к-масштабированию-каталога-до-10-миллионов-записей)
- [Структура проекта](#структура-проекта)
- [Тестирование](#тестирование)
- [Дополнительное задание: Tombstone](#дополнительное-задание-tombstone)

---

## Стек

| Слой | Технология |
|---|---|
| Runtime | Rust 1.85 (stable), Tokio |
| HTTP | Axum 0.7 |
| База данных | PostgreSQL 16 |
| ORM | SeaORM 1.x, SeaQuery |
| Миграции | SeaORM Migration (отдельный crate) |
| Логирование | tracing, tracing-subscriber |
| Контейнеризация | Docker, Docker Compose V2 |

Дополнительно: `pg_trgm` для подстрочного поиска, `tower-http` (CORS), `serde`, `chrono`, `rust_decimal`, `uuid`.

---

## 1. Архитектура

| Слой | Ответственность | Может импортировать |
|---|---|---|
| `shared` | `Page<T>`, общие типы | только внешние крейты (без axum/sea-orm) |
| `dom` | Доменные модели, `PluginRepo`, `PluginUoW`, `PluginError`, `SearchCriteria`, `SortOrder` | `shared` |
| `app` | `PluginSearchService`, `AppState`, трейт `PluginSearch` | `dom`, `shared` |
| `infra` | SeaORM entity, `RepoImpl<'a, C: ConnectionTrait>`, `AppCtx`, миграции | `dom`, `app`, `shared` |
| `present` | Axum-хендлеры, DTO, `ApiError` | `app`, `shared` |
| `main.rs` + `config.rs` | Composition root, сборка зависимостей, конфиг | всё |

---

## 2. Индексы PostgreSQL

Миграции: `migration/src/m20250101_000002_create_indexes.rs`, `m20250101_000003_add_tombstone.rs`.

| Индекс | Тип | Колонки / выражение | Назначение |
|---|---|---|---|
| `plugins_name_trgm_idx` | GIN + `gin_trgm_ops` | `name` | `name ILIKE '%q%'` |
| `plugins_description_trgm_idx` | GIN + `gin_trgm_ops` | `description` | `description ILIKE '%q%'` |
| `plugins_category_idx` | B-tree | `category` | `category = $n` |
| `plugins_price_idx` | B-tree | `price` | `price >= / <=`, `ORDER BY price` |
| `plugins_rating_idx` | B-tree | `rating` | `ORDER BY rating DESC` |
| `plugins_downloads_idx` | B-tree | `downloads` | `ORDER BY downloads DESC` |
| `plugins_updated_at_idx` | B-tree | `updated_at` | `ORDER BY updated_at DESC` |
| `plugins_tags_gin_idx` | GIN | `tags` | `tags @> ARRAY[...]` (AND-семантика) |
| `plugins_published_idx` | B-tree, **partial** | `(updated_at DESC, id)` `WHERE deleted_at IS NULL AND status = 'published'::plugin_status` | Отсекает draft/archived/tombstones |
| `plugins_deleted_at_idx` | B-tree, **partial** | `deleted_at` `WHERE deleted_at IS NOT NULL` | Фоновая очистка tombstones |

---

## 3. Стратегия поиска

Реализовано в `RepoImpl::search` (`infra/db/persist/plugins/repo.rs`). Собираются **два** параллельных запроса через `tokio::try_join!`:

| Запрос | Что делает | Особенности |
|---|---|---|
| **items** | `SELECT ... ORDER BY ... LIMIT ... OFFSET ...` | Возвращает страницу |
| **count** | `SELECT COUNT(*)` с тем же `WHERE`, без `LIMIT/OFFSET` | Для корректного `total` |

Вся фильтрация и пагинация на стороне PostgreSQL; в память Rust попадает только страница результатов.

### WHERE (`build_where`)

**Всегда добавляется:**

| Условие | Назначение |
|---|---|
| `status = 'published'::plugin_status` | Только опубликованные |
| `deleted_at IS NULL` | Tombstones невидимы |

**Опциональные условия по параметрам:**

| Параметр | SQL |
|---|---|
| `q` | `(name ILIKE $n OR description ILIKE $n)` с паттерном `%escaped(q)%` |
| `category` | `category = $n` |
| `tags` | `tags @> $n` (`text[]`, AND по всем указанным) |
| `min_price` / `max_price` | `price >= $n` / `price <= $n` |
| `min_rating` | `rating >= $n` |

**Экранирование LIKE:**

| Символ | Экранируется в | Зачем |
|---|---|---|
| `\` | `\\` | Литерал backslash |
| `%` | `\%` | Литерал `%`, не wildcard |
| `_` | `\_` | Литерал `_`, не wildcard |

### ORDER BY (`build_order`)

Все сортировки **детерминированы**: к любому `ORDER BY` добавляется `id ASC` как tie-breaker.

| `sort` | ORDER BY |
|---|---|
| `relevance` + `q` | `CASE WHEN name ILIKE '%q%' THEN 0 ELSE 1 END ASC, id ASC` |
| `relevance` без `q` | `id ASC` |
| `price_asc` | `price ASC, id ASC` |
| `price_desc` | `price DESC, id ASC` |
| `rating_desc` | `rating DESC, id ASC` |
| `downloads_desc` | `downloads DESC, id ASC` |
| `newest` | `updated_at DESC, id ASC` |

**Relevance.** Требование ТЗ - «приоритет совпадений в названии над совпадениями только в описании». Реализовано через `CASE WHEN`: `name`-совпадение -> 0 (сортируется первым), иначе -> 1. Это не «настоящий» relevance score, а минимально достаточная реализация для 100k-1M. Для 10M - переход на FTS `ts_rank` (см. раздел 6).

### LIMIT/OFFSET

| Параметр | Ограничение | Где валидируется |
|---|---|---|
| `limit` | `1..100` | `dom::SearchCriteria::validate` |
| `page` | `≥ 1` | `dom::SearchCriteria::validate` |
| `OFFSET` | `(page - 1) * limit` | `RepoImpl::search` |

Клиент с `page=0` или `limit=1000` получает `400` - до вызова репозитория.

## EXPLAIN (ANALYZE, BUFFERS)

Замеры - на **100 000 плагинов** (`seed --count 100000 --seed 42`), после `ANALYZE plugins`.
Оборудование: локальная машина, PostgreSQL 16 в Docker.

### Сводная таблица

| # | Запрос | Execution Time | Ключевой узел |
|---|---|---|---|
| 1 | `q='export'`, `sort=downloads_desc`, LIMIT 20 | **1.3 ms** | `Index Scan Backward using plugins_downloads_idx` |
| 2 | `q='export'`, `sort=relevance`, LIMIT 20 | **8.6 ms** | `BitmapOr` по trgm-индексам |
| 3 | `category=productivity`, `price 1000..5000`, `sort=rating_desc`, LIMIT 20 | **0.5 ms** | `Index Scan Backward using plugins_rating_idx` |
| 4 | `tags @> ['pdf','export']`, `sort=downloads_desc`, LIMIT 20 | **1.3 ms** | `Index Scan Backward using plugins_downloads_idx` |
| 5 | `COUNT(*)` для `q='export'` | **4.6 ms** | `BitmapOr` + `Bitmap Heap Scan` |

Все запросы < 10 ms на 100k. Запас для 10-кратного роста.

### Query 1: q + downloads_desc

```sql
SELECT id, name, description, category, tags, price, rating, downloads, status, updated_at
FROM plugins
WHERE status = 'published' AND deleted_at IS NULL
  AND (name ILIKE '%export%' OR description ILIKE '%export%')
ORDER BY downloads DESC, id ASC
LIMIT 20;
```

```
Limit  (actual time=1.270..1.273 rows=20 loops=1)
  ->  Incremental Sort  (Sort Key: downloads DESC, id; Presorted Key: downloads)
        ->  Index Scan Backward using plugins_downloads_idx on plugins
              Filter: ((status = 'published'::plugin_status)
                       AND (deleted_at IS NULL)
                       AND ((name ~~* '%export%'::text) OR (description ~~* '%export%'::text)))
              Rows Removed by Filter: 372
Execution Time: 1.304 ms
```

**Что происходит.** Планировщик **не использует GIN trgm**, хотя они есть. Причина - `LIMIT 20` + `ORDER BY downloads DESC` позволяет стратегию top-N: идём по готовому порядку `downloads`, фильтруем на ходу, останавливаемся на 20-й подходящей строке. Просмотрено **393 строки**, а не 100 000.

`Incremental Sort` использует готовый порядок `downloads`, досортировывая только tie-breaker по `id`. `Full-sort Groups: 1` - все 20 строк в одной группе `downloads`, досортировка по `id` тривиальна.

### Query 2: q + relevance

```sql
... ORDER BY CASE WHEN name ILIKE '%export%' THEN 0 ELSE 1 END ASC, id ASC LIMIT 20;
```

```
Limit  (actual time=8.507..8.510 rows=20 loops=1)
  ->  Sort  (Sort Key: CASE WHEN name ILIKE '%export%' THEN 0 ELSE 1 END, id)
        Sort Method: top-N heapsort  Memory: 39kB
        ->  Bitmap Heap Scan on plugins  (rows=4394)
              Recheck Cond: ((name ~~* '%export%') OR (description ~~* '%export%'))
              Filter: (status = 'published'::plugin_status AND deleted_at IS NULL)
              Rows Removed by Filter: 1785
              ->  BitmapOr
                    ->  Bitmap Index Scan on plugins_name_trgm_idx (rows=6179)
                    ->  Bitmap Index Scan on plugins_description_trgm_idx (rows=0)
Execution Time: 8.592 ms
```

**Что происходит.** Relevance не совпадает ни с одним индексом -> нужно собрать все совпадения и отсортировать. Планировщик использует оба GIN trgm-индекса через `BitmapOr`:

- `name_trgm_idx` нашёл **6179 строк**;
- `description_trgm_idx` - **0** (в сидере ключевые слова только в `name`).

`Bitmap Heap Scan` читает 2949 страниц, применяет `status`-фильтр, отбрасывает 1785, возвращает 4394. `Sort Method: top-N heapsort` - сортировка 39 kB в памяти, `LIMIT 20` реализован через кучу: Postgres не сортирует все 4394 строки, а поддерживает top-20.

### Query 3: category + price + rating_desc

```sql
... WHERE status = 'published' AND deleted_at IS NULL
  AND category = 'productivity' AND price BETWEEN 1000 AND 5000
ORDER BY rating DESC, id ASC LIMIT 20;
```

```
Limit  (actual time=0.523..0.525 rows=20 loops=1)
  ->  Incremental Sort  (Sort Key: rating DESC, id; Presorted Key: rating)
        ->  Index Scan Backward using plugins_rating_idx on plugins
              Filter: (price >= 1000 AND price <= 5000 AND status = 'published'
                       AND deleted_at IS NULL AND category = 'productivity')
              Rows Removed by Filter: 878
Execution Time: 0.532 ms
```

**Что происходит.** Та же top-N стратегия: идём по `rating`, фильтруем. Просмотрено **905 строк**, отброшено 878.

**Почему не `plugins_category_idx`.** Категория `productivity` - одна из 6, ~16 000 строк. Это неселективно. Пройти top-20 по `rating` дешевле, чем читать 16k и сортировать.

### Query 4: tags AND

```sql
... WHERE status = 'published' AND deleted_at IS NULL
  AND tags @> ARRAY['pdf','export']::text[]
ORDER BY downloads DESC, id ASC LIMIT 20;
```

```
Limit  (actual time=1.242..1.244 rows=20 loops=1)
  ->  Incremental Sort
        ->  Index Scan Backward using plugins_downloads_idx
              Filter: (tags @> '{pdf,export}'::text[] AND status = 'published'
                       AND deleted_at IS NULL)
              Rows Removed by Filter: 2042
Execution Time: 1.252 ms
```

Снова top-N через `downloads`. GIN по тегам не используется из-за `LIMIT` - но при `COUNT(*)` без `LIMIT` он подхватится (см. Query 5).

### Query 5: count

```sql
SELECT COUNT(*) FROM plugins
WHERE status = 'published' AND deleted_at IS NULL
  AND (name ILIKE '%export%' OR description ILIKE '%export%');
```

```
Aggregate  (actual time=4.611..4.611 rows=1 loops=1)
  ->  Bitmap Heap Scan on plugins  (rows=4394)
        Recheck Cond: ((name ~~* '%export%') OR (description ~~* '%export%'))
        Filter: (status = 'published' AND deleted_at IS NULL)
        ->  BitmapOr
              ->  Bitmap Index Scan on plugins_name_trgm_idx
              ->  Bitmap Index Scan on plugins_description_trgm_idx
Execution Time: 4.647 ms
```

`COUNT(*)` не может применить top-N - считает все. Использует тот же `BitmapOr`, что и Query 2, но без `Sort`.

### Общие выводы

| # | Наблюдение | Что это значит |
|---|---|---|
| 1 | **GIN trgm подхватывается** там, где нет подходящего B-tree для сортировки (Query 2, Query 5) | Индексы работают так, как задумано |
| 2 | **B-tree + top-N выигрывает** для `ORDER BY <индексируемое поле>` + `LIMIT` (Query 1, 3, 4) | Planner выбирает оптимальную стратегию, не всегда trgm |
| 3 | **`Incremental Sort`** появляется, когда первое поле `ORDER BY` уже отсортировано индексом | Правильный порядок колонок `(downloads, id)` в индексе и `ORDER BY downloads, id` |
| 4 | **Partial index `plugins_published_idx`** в этих запросах не использовался | На 100k planner предпочитает `downloads_idx` или trgm; на 1M+ стоит рассмотреть partial GIN trgm |
| 5 | **Все запросы < 10 ms** на 100k | Запас для 10-кратного роста есть |

**Что улучшится на 10M** (см. раздел 6):

| Индекс | Эффект на 10M |
|---|---|
| Partial GIN trgm | Query 2 ускорится в 2–3 раза (индекс в 2–3 раза меньше) |
| FTS `tsvector` + GIN | Query 2 и Query 5 - в 10–20 раз быстрее |
| Cursor pagination | Глубокие страницы (page=5000) - с ~500 ms до ~10 ms |

---

## 4. Ограничения OFFSET-пагинации

Текущая пагинация - `LIMIT/OFFSET`. Имеет следующие ограничения:

### (а) Линейная деградация на глубине

```sql
SELECT ... FROM plugins
WHERE ... ORDER BY ... LIMIT 20 OFFSET 10000;
```

Postgres **просканирует и отбросит 10 000 строк**, чтобы вернуть 20. Сложность - `O(offset + limit)`, не `O(limit)`.

**Таблица деградации (на каталоге 100k, `limit=20`):**

| `page` | `OFFSET` | Ожидаемое время |
|---|---|---|
| 1 | 0 | ~10 ms |
| 100 | 1 980 | ~15 ms |
| 500 | 9 980 | ~50 ms |
| 10 000 | 199 980 | ~500 ms |
| 100 000 | 1 999 980 | секунды / таймаут |

### (б) Нестабильность при конкурентных вставках

Между запросом `page=1` и `page=2` может быть вставлена новая запись.

**Поведение по сортировкам при вставке:**

| `sort` | Действие | Итог |
|---|---|---|
| `newest` | Новая запись сдвигает всё вниз | Клиент **видит запись дважды** (page=1 и page=2) |
| `price_asc` | Новая запись сдвигает всё вниз | Может **пропустить** одну запись |
| `downloads_desc` | Сдвиг только если у новой записи больше downloads | Обычно стабильно |

OFFSET не даёт гарантии на непротиворечия между страницами без транзакции на весь обход.

### Что уже сделано

| Мера | Эффект |
|---|---|
| `limit <= 100` | Клиент не вытянет всё одним запросом; глубокий offset недостижим через `limit` |
| `total` отдельным запросом | Клиент видит реальное количество страниц, не «долистывает» до пустых |
| Tie-breaker `id ASC` | При равных значениях сортируемого поля порядок стабилен в пределах snapshot |
| Partial index `(updated_at DESC, id)` | Для `sort=newest` глубокий offset дешевле |

### Что делать на 10M+ (не реализовано)

**Cursor-based pagination** - замена `OFFSET`:

```sql
-- вместо: WHERE ... ORDER BY downloads DESC, id ASC LIMIT 20 OFFSET 10000
WHERE (downloads, id) < ($cursor_downloads, $cursor_id)
ORDER BY downloads DESC, id ASC
LIMIT 20;
```

`$cursor` - непрозрачная base64-строка `(последнее_значение_поля, id)`, возвращаемая клиенту как `next_cursor`. Сложность - `O(limit)`, не зависит от глубины.

**Применимость cursor по сортировкам:**

| `sort` | Применяемость | Причина |
|---|---|---|
| `newest` | Да | `updated_at` - колонка таблицы |
| `downloads_desc` | Да | `downloads` - колонка таблицы |
| `price_asc` / `price_desc` | Да | `price` - колонка таблицы |
| `rating_desc` | Да | `rating` - колонка таблицы |
| `relevance` + `q` | **Нет** | `CASE WHEN` - не колонка, нельзя построить `WHERE (relevance, id) > (X, Y)` |

Для `relevance` - оставить OFFSET с ограничением `page <= 100`.

---

## 5. Особенности работы с SeaORM

### Entities - только в `infra`

**Расположение типов и маппинга:**

| Тип | Файл | Назначение |
|---|---|---|
| `entity::Model` | `infra/db/entities/plugin.rs` | SeaORM-модель таблицы, `DeriveEntityModel` |
| `dom::Plugin` | `dom/plugins/model.rs` | «Чистая» структура без SeaORM-макросов |
| `From<entity::Model> for Plugin` | `infra/db/persist/plugins/mapping.rs` | БД -> домен |
| `to_active_model(&Plugin) -> ActiveModel` | `infra/db/persist/plugins/mapping.rs` | Домен -> БД (insert/update) |

### Нативный PostgreSQL ENUM

```rust
#[derive(EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "plugin_status")]
pub enum PluginStatus {
    #[sea_orm(string_value = "draft")] Draft,
    #[sea_orm(string_value = "published")] Published,
    #[sea_orm(string_value = "archived")] Archived,
}
```

**Параметры макроса:**

| Параметр | Значение | Почему |
|---|---|---|
| `rs_type` | `"String"` | Rust-представление - строка |
| `db_type` | `"Enum"` | **Не** `"String(Some(10))"`. С `Enum` SeaORM генерирует `$1::plugin_status` - partial index подхватывается |
| `enum_name` | `"plugin_status"` | Должно **буквально** совпадать с `CREATE TYPE plugin_status AS ENUM` из миграции |

### `RepoImpl<'a, C: ConnectionTrait>`

**Два конструктора:**

| Конструктор | Что принимает | Используется в |
|---|---|---|
| `from_db(&DatabaseConnection)` | Пул соединений | `AppCtx::plugins()` - одиночные операции |
| `from_tx(&DatabaseTransaction)` | Транзакция | внутри `AppCtx::transaction(...)` |

`ConnectionTrait` покрывает оба типа - код `search`/`upsert`/`delete` не дублируется.

### Массивы `TEXT[]`

Требует фичу `postgres-array`:

```toml
sea-orm = { version = "1", features = [
    "sqlx-postgres", "runtime-tokio-rustls", "macros", "postgres-array"
] }
```

Без неё `tags: Vec<String>` не маппится в `TEXT[]` - генератор выдаст `String`, компиляция упадёт на первом `INSERT`.

### Raw SQL там, где ORM мешает

**Что через raw SQL:**

| Выражение | Почему не ORM |
|---|---|
| `name ILIKE $1` | ORM-обёртка громоздкая, генерирует менее предсказуемый SQL |
| `description ILIKE $1` | То же |
| `tags @> $1` | `ArrayType::String` + `Expr::cust_with_values` проще |
| `CASE WHEN name ILIKE $1 THEN 0 ELSE 1 END` | ORM не имеет удобного API для `CASE` в `ORDER BY` |

Параметры `$1` экранируются драйвером - SQL-инъекций нет. ТЗ это допускает: «допускать raw SQL через SeaORM там, где ORM не обеспечивает удобной или эффективной реализации».

### `insert_many` для сидера

| Параметр | Значение | Почему |
|---|---|---|
| Размер батча | 1000 | 10 колонок × 1000 = 10 000 параметров, лимит Postgres - 65535 |
| Метод | `Entity::insert_many(batch).exec(conn)` | Один `INSERT` с тысячей `VALUES` |
| Эффект | 100k за секунды | Без батчинга - минуты |

---

## 6. Подход к масштабированию каталога до 10 миллионов записей

Текущая реализация рассчитана на 100k-1M. Для 10M - следующие шаги, в порядке приоритета.

**Список изменений:**

| # | Изменение | Приоритет | Что даёт |
|---|---|---|---|
| 1 | Cursor-based pagination | Обязательно | `O(limit)` вместо `O(offset+limit)` |
| 2 | Partial GIN trgm по published | Высокий | Индекс в 2-3 раза меньше |
| 3 | FTS `tsvector` + GIN | Высокий | В 10-20 раз быстрее substring-поиска |
| 4 | Партиционирование по `status` | Средний | Скан только 1/3 таблицы |
| 5 | Кеш популярных запросов | Средний | 90% трафика из памяти |
| 6 | Read-replicas + PgBouncer | Средний | Горизонтальное масштабирование read |
| 7 | Профилирование (`pg_stat_statements`, `auto_explain`) | Обязательно | Видимость регрессий |

### 6.1. Cursor-based pagination

OFFSET на 10M деградирует до секунд. Заменить на cursor (см. раздел 4). Для `relevance` - оставить OFFSET с ограничением `page ≤ 100`.

**Оценка выигрыша:**

| Метрика | OFFSET | Cursor |
|---|---|---|
| Сложность | `O(offset + limit)` | `O(limit)` |
| Время на page=1 | ~10 ms | ~10 ms |
| Время на page=500 | ~50 ms | ~10 ms |
| Время на page=5000 | ~500 ms | ~10 ms |

### 6.2. Partial GIN trgm по published

```sql
CREATE INDEX plugins_name_trgm_published_idx ON plugins USING gin (name gin_trgm_ops)
    WHERE status = 'published' AND deleted_at IS NULL;
CREATE INDEX plugins_description_trgm_published_idx ON plugins USING gin (description gin_trgm_ops)
    WHERE status = 'published' AND deleted_at IS NULL;
```

**Эффект по объёму:**

| Каталог | Размер GIN (полный) | Размер GIN (partial) |
|---|---|---|
| 100k | ~40 MB | ~28 MB |
| 1M | ~400 MB | ~280 MB |
| 10M | ~4 GB | ~2.8 GB |

Запрос должен **точно** содержать `status = 'published' AND deleted_at IS NULL` - иначе partial не используется.

### 6.3. Full Text Search вместо `pg_trgm`

```sql
ALTER TABLE plugins ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', name), 'A') ||
        setweight(to_tsvector('simple', description), 'B')
    ) STORED;

CREATE INDEX plugins_search_vector_idx ON plugins USING gin (search_vector)
    WHERE status = 'published' AND deleted_at IS NULL;
```

```sql
WHERE search_vector @@ to_tsquery('simple', 'export:*')
ORDER BY ts_rank(search_vector, query) DESC, id ASC
```

**Сравнение подходов:**

| Подход | `q='export'` на 10M | Произвольные подстроки | Ранжирование |
|---|---|---|---|
| `pg_trgm` | ~460 ms | Да (`%xpor%`) | `CASE WHEN` (бинарь) |
| FTS `ts_rank` | ~40 ms | **Нет** (только префиксы слов) | `ts_rank` (число) |
| Гибрид | ~40 ms на словесных, `pg_trgm` на substring | Да | По ситуации |

`setweight('A')` для `name`, `'B'` для `description` - «name важнее description» штатными средствами Postgres.

### 6.4. Партиционирование

```sql
CREATE TABLE plugins (...) PARTITION BY LIST (status);
CREATE TABLE plugins_published PARTITION OF plugins FOR VALUES IN ('published');
CREATE TABLE plugins_draft     PARTITION OF plugins FOR VALUES IN ('draft');
CREATE TABLE plugins_archived  PARTITION OF plugins FOR VALUES IN ('archived');
```

**Плюсы/минусы:**

| Аспект | List по `status` | Range по `updated_at` |
|---|---|---|
| Скан по `status='published'` | Только 1/3 таблицы | Все партиции |
| `sort=newest` | Все партиции | Только «горячие» |
| Detach старых данных | Сложно | Легко (detach по месяцу) |
| GIN `tags` | На каждой партиции | На каждой партиции |

### 6.5. Кеш популярных запросов

```rust
pub trait SearchCache: Send + Sync {
    async fn get(&self, key: &str) -> Option<Page<Plugin>>;
    async fn put(&self, key: &str, page: Page<Plugin>, ttl: Duration);
    async fn invalidate(&self, key: &str);
}
```

**Параметры кеша:**

| Параметр | Значение | Обоснование |
|---|---|---|
| TTL | 5-30 с | Данные обновляются через `upsert`, не из пользовательских запросов |
| Реализация | `moka` (in-process) или Redis (shared) | In-process - для одной реплики; Redis - для нескольких |
| Инвалидация | По префиксу категории или полная при `upsert`/`delete` | Баланс между свежестью и hit rate |
| Single-flight | `tokio::sync::OnceCell` | Защита от thundering herd на истечении TTL |
| Ключ | Канонизированный `SearchCriteria` | `Option`-поля нормализуются, теги сортируются |

### 6.6. Read-replicas + PgBouncer

```rust
// composition root
let write_db = Database::connect(write_url).await?;
let read_db  = Database::connect(read_url).await?;
let write_ctx = AppCtx::new(write_db);
let read_ctx  = AppCtx::new(read_db);
// search -> read_ctx, upsert/delete -> write_ctx
```

**Распределение нагрузки:**

| Операция | Куда идёт | Доля трафика |
|---|---|---|
| `search` | read-replica | ~90% |
| `upsert` | writer | ~5% |
| `delete` | writer | ~5% |

`RepoImpl<'a, C: ConnectionTrait>` уже generic - менять реализацию не придётся, только проводку в `main.rs`.

### 6.7. Профилирование и мониторинг

| Инструмент | Что даёт |
|---|---|
| `pg_stat_statements` | Топ медленных запросов (нормализованные) |
| `auto_explain` | Автологирование планов > 100 ms |
| `EXPLAIN (ANALYZE, BUFFERS)` | Проверка после каждого изменения индексов |
| `ANALYZE plugins` | Обновление статистики после массовых загрузок |
| `work_mem` | Для `top-N heapsort` на больших выборках |
| `shared_buffers` | 25% RAM |
| `effective_cache_size` | 50-75% RAM |

### Итог: эволюция по объёму

| Записей | Пагинация | Поиск | Индексы | Инфраструктура |
|---|---|---|---|---|
| 100k | OFFSET | `pg_trgm` | текущие | 1 инстанс |
| 1M | OFFSET | `pg_trgm` | partial GIN trgm | 1 инстанс + replica |
| 10M | **cursor** | **FTS + trgm (гибрид)** | partial GIN trgm + FTS + partition | writer + N replicas + PgBouncer + кеш |

---

## Структура проекта

```
.
├── Cargo.toml / Cargo.lock
├── Dockerfile              # multi-stage: builder + server + migration
├── docker-compose.yml      # postgres + migration + server
├── .env.example
├── manual_test.html        # ручное тестирование (поиск / PUT / DELETE)
├── migration/              # отдельный crate
│   └── src/
│       ├── lib.rs
│       ├── main.rs                          # CLI: up / down / status / fresh
│       ├── m20250101_000001_create_plugins.rs
│       ├── m20250101_000002_create_indexes.rs
│       └── m20250101_000003_add_tombstone.rs
└── server/
    ├── src/
    │   ├── lib.rs
    │   ├── main.rs                          # composition root
    │   ├── config.rs
    │   ├── shared/mod.rs                    # Page<T>
    │   ├── dom/plugins/                     # model, repo, uow, errors
    │   ├── app/                             # state, services, test_mocks
    │   ├── infra/db/                        # ctx, entities, persist/plugins
    │   ├── present/http/                    # routes, plugins, error
    │   └── bin/                             # seed, probe
    └── tests/integration.rs                 # 10 сценариев ТЗ + extras + tombstone
```

### Тестирование

**Типы тестов:**

| Тип | Файл | Требует БД | Что проверяет |
|---|---|---|---|
| Юнит `app` | `server/src/app/services/plugin_search_service.rs` (`#[cfg(test)]`) | Нет (`InMemoryPluginRepo`) | Валидация, идемпотентность, только published |
| Юнит `present` | `server/src/present/http/plugins.rs` (`#[cfg(test)]`) | Нет (`MockSearch`) | Парсинг, `ApiError`-маппинг, HTTP-статусы |
| Интеграционные | `server/tests/integration.rs` | Да (`plugin_search_test`) | 10 сценариев ТЗ + tombstone |

**Команды:**

```bash
# Юнит-тесты (без БД)
cargo test -p server app::
cargo test -p server present::

# Интеграционные
docker compose exec postgres psql -U postgres -c "CREATE DATABASE plugin_search_test;"
DATABASE_URL=postgres://postgres:postgres@localhost:5432/plugin_search_test \
    cargo test -p server --test integration -- --test-threads=1
```

---

### Дополнительное задание: Tombstone

**Поведение по сценариям:**

| Сценарий | `deleted_at` | `incoming.updated_at` vs `deleted_at` | Результат | HTTP |
|---|---|---|---|---|
| Записи не было | - | - | `Created`, запись живая | 201 |
| Обычное обновление | `NULL` | `>` existing | `Updated` | 200 |
| Идентичное повторное | `NULL` | `=` и контент идентичен | `NoOp` | 200 |
| Устаревшее обновление | `NULL` | `<` existing | `Stale` | 409 |
| Конфликт | `NULL` | `=` и контент разный | `Conflict` | 409 |
| Запоздавшее после delete | `NOT NULL` | `<= deleted_at` | `DeletedStale` | 409 |
| Осознанное пересоздание | `NOT NULL` | `> deleted_at` | `Updated`, `deleted_at = NULL` | 200 |

**SQL-эффекты:**

| Операция | SQL |
|---|---|
| `DELETE` | `UPDATE plugins SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL` |
| `search` | `WHERE ... AND deleted_at IS NULL` (в `build_where`) |
| Partial index | `WHERE deleted_at IS NULL AND status = 'published'::plugin_status` |
| Очистка (cron) | `DELETE FROM plugins WHERE deleted_at < now() - interval '30 days'` |
```

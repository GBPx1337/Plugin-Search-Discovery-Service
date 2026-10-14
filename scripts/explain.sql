-- Снимаем планы для типичных запросов.
-- Запуск: docker compose exec -T postgres psql -U postgres -d plugin_search < scripts/explain.sql

\echo '=== Query 1: q + downloads_desc ==='
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, name, description, category, tags, price, rating, downloads, status, updated_at
FROM plugins
WHERE status = 'published'
  AND (name ILIKE '%export%' OR description ILIKE '%export%')
ORDER BY downloads DESC, id ASC
    LIMIT 20;

\echo '=== Query 2: q + relevance ==='
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, name, description, category, tags, price, rating, downloads, status, updated_at
FROM plugins
WHERE status = 'published'
  AND (name ILIKE '%export%' OR description ILIKE '%export%')
ORDER BY CASE WHEN name ILIKE '%export%' THEN 0 ELSE 1 END ASC, id ASC
    LIMIT 20;

\echo '=== Query 3: category + price + rating_desc ==='
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, name, description, category, tags, price, rating, downloads, status, updated_at
FROM plugins
WHERE status = 'published'
  AND category = 'productivity'
  AND price BETWEEN 1000 AND 5000
ORDER BY rating DESC, id ASC
    LIMIT 20;

\echo '=== Query 4: tags AND ==='
EXPLAIN (ANALYZE, BUFFERS)
SELECT id, name, description, category, tags, price, rating, downloads, status, updated_at
FROM plugins
WHERE status = 'published'
  AND tags @> ARRAY['pdf', 'export']::text[]
ORDER BY downloads DESC, id ASC
    LIMIT 20;

\echo '=== Query 5: count for total ==='
EXPLAIN (ANALYZE, BUFFERS)
SELECT COUNT(*)
FROM plugins
WHERE status = 'published'
  AND (name ILIKE '%export%' OR description ILIKE '%export%');
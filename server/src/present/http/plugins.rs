use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use axum::extract::Path;
use chrono::{DateTime, FixedOffset};
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::app::state::AppState;
use crate::dom::plugins::errors::PluginError;
use crate::dom::plugins::model::{Plugin, PluginId, PluginStatus, SearchCriteria, SortOrder, UpsertOutcome, MAX_LIMIT};
use crate::shared::Page;

/// DTO query-параметров поиска.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    #[serde(default, deserialize_with = "deserialize_tags")]
    pub tags: Vec<String>,
    pub min_price: Option<i32>,
    pub max_price: Option<i32>,
    pub min_rating: Option<f64>,
    pub sort: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct IndexPluginRequest {
    pub name: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub price: i32,
    pub rating: rust_decimal::Decimal,
    pub downloads: i64,
    pub status: String,
    pub updated_at: DateTime<FixedOffset>,
}

#[derive(Debug, Serialize)]
pub struct IndexResponse {
    pub outcome: &'static str,
}

fn deserialize_tags<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw {
        None => Ok(vec![]),
        Some(s) => Ok(s
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()),
    }
}

/// DTO одного плагина в ответе.
#[derive(Debug, Serialize)]
pub struct PluginResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub price: i32,
    pub rating: f64,
    pub downloads: i64,
    pub status: String,
    pub updated_at: String,
}

/// DTO пагинированного ответа.
#[derive(Debug, Serialize)]
pub struct PageResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
    pub total_pages: u32,
}

/// Хендлер `GET /api/v1/plugins/search`.
pub async fn search_plugins(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<PageResponse<PluginResponse>>, ApiError> {

    // 1. Парсинг query → SearchCriteria
    let criteria = SearchCriteria {
        q: query.q,
        category: query.category,
        tags: query.tags,
        min_price: query.min_price,
        max_price: query.max_price,
        min_rating: query
            .min_rating
            .map(|r| rust_decimal::Decimal::from_f64_retain(r).unwrap_or_default()),
        sort: parse_sort(&query.sort)?,
        page: query.page.unwrap_or(1),
        limit: query.limit.unwrap_or(20).min(MAX_LIMIT),
    };

    // 2. Вызов сервиса (валидация происходит внутри `search`)
    let page: Page<Plugin> = state
        .plugin_search
        .search(criteria)
        .await
        .map_err(ApiError::from)?;

    let total_pages= page.total_pages();
    // 3. Конвертация доменных моделей в DTO
    let items = page.items.into_iter().map(PluginResponse::from).collect();
    let response = PageResponse {
        items,
        total: page.total,
        page: page.page,
        limit: page.limit,
        total_pages,
    };

    Ok(Json(response))
}

/// Парсит строку сортировки в `SortOrder`.
fn parse_sort(sort: &Option<String>) -> Result<SortOrder, ApiError> {
    match sort.as_deref() {
        None | Some("relevance") => Ok(SortOrder::Relevance),
        Some("price_asc") => Ok(SortOrder::PriceAsc),
        Some("price_desc") => Ok(SortOrder::PriceDesc),
        Some("rating_desc") => Ok(SortOrder::RatingDesc),
        Some("downloads_desc") => Ok(SortOrder::DownloadsDesc),
        Some("newest") => Ok(SortOrder::Newest),
        Some(other) => Err(ApiError::validation(format!(
            "unknown sort order: {other}"
        ))),
    }
}

pub async fn index_plugin(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<IndexPluginRequest>,
) -> Result<(StatusCode, Json<IndexResponse>), ApiError> {
    let status = match body.status.as_str() {
        "draft" => PluginStatus::Draft,
        "published" => PluginStatus::Published,
        "archived" => PluginStatus::Archived,
        other => {
            return Err(ApiError::validation(format!(
                "unknown status: {other}"
            )))
        }
    };

    let plugin = Plugin {
        id: PluginId::new(id),
        name: body.name,
        description: body.description,
        category: body.category,
        tags: body.tags,
        price: body.price,
        rating: body.rating,
        downloads: body.downloads,
        status,
        updated_at: body.updated_at,
    };

    let outcome = state
        .plugin_search
        .index_plugin(plugin)
        .await
        .map_err(ApiError::from)?;

    let (http_status, label) = match outcome {
        UpsertOutcome::Created => (StatusCode::CREATED, "created"),
        UpsertOutcome::Updated => (StatusCode::OK, "updated"),
        UpsertOutcome::NoOp => (StatusCode::OK, "noop"),
        UpsertOutcome::Stale => (StatusCode::CONFLICT, "stale"),
        UpsertOutcome::Conflict => (StatusCode::CONFLICT, "conflict"),
    };

    Ok((http_status, Json(IndexResponse { outcome: label })))
}

pub async fn delete_plugin(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    state
        .plugin_search
        .delete_plugin(PluginId::new(id))
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ошибка API с HTTP-статусом.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn validation(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
        }
    }
}

/// Маппинг доменных ошибок в HTTP-статусы (Шаг 12 из плана).
impl From<PluginError> for ApiError {
    fn from(err: PluginError) -> Self {
        match err {
            PluginError::NotFound(_) => Self {
                status: StatusCode::NOT_FOUND,
                message: err.to_string(),
            },
            PluginError::Validation(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message: err.to_string(),
            },
            PluginError::Conflict(_) | PluginError::Stale { .. } => Self {
                status: StatusCode::CONFLICT,
                message: err.to_string(),
            },
            PluginError::Internal(_) => Self {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "internal server error".into(),
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

/// Маппинг доменной модели в DTO ответа.
impl From<Plugin> for PluginResponse {
    fn from(p: Plugin) -> Self {
        Self {
            id: p.id.into_inner(),
            name: p.name,
            description: p.description,
            category: p.category,
            tags: p.tags,
            price: p.price,
            rating: p.rating.to_f64().unwrap_or(0.0),
            downloads: p.downloads,
            status: format!("{:?}", p.status).to_lowercase(),
            updated_at: p.updated_at.to_rfc3339(),
        }
    }
}


#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::body::Body;
    use axum::extract::Path;
    use axum::http::{Request, StatusCode};
    use chrono::{DateTime, FixedOffset, Utc};
    use rust_decimal::Decimal;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::app::state::{AppState, PluginSearch};
    use crate::dom::plugins::errors::PluginError;
    use crate::dom::plugins::model::{Plugin, PluginId, PluginStatus, SearchCriteria, SortOrder, UpsertOutcome, MAX_LIMIT};
    use crate::present::http::routes;
    use crate::shared::Page;


    struct MockSearch {
        received: Mutex<Option<SearchCriteria>>,
        result: Mutex<Option<Result<Page<Plugin>, PluginError>>>,
    }

    impl MockSearch {
        fn ok(page: Page<Plugin>) -> Arc<Self> {
            Arc::new(Self {
                received: Mutex::new(None),
                result: Mutex::new(Some(Ok(page))),
            })
        }

        fn err(e: PluginError) -> Arc<Self> {
            Arc::new(Self {
                received: Mutex::new(None),
                result: Mutex::new(Some(Err(e))),
            })
        }

        fn received(&self) -> Option<SearchCriteria> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl PluginSearch for MockSearch {
        async fn search(
            &self,
            criteria: SearchCriteria,
        ) -> Result<Page<Plugin>, PluginError> {
            *self.received.lock().unwrap() = Some(criteria);
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("MockSearch::search called more than once")
        }

        async fn index_plugin(&self, plugin: Plugin) -> Result<UpsertOutcome, PluginError> {
            todo!()
        }

        async fn delete_plugin(&self, id: PluginId) -> Result<(), PluginError> {
            todo!()
        }
    }


    fn build_router(mock: Arc<MockSearch>) -> axum::Router {
        let search: Arc<dyn PluginSearch> = mock;
        let state = AppState {
            plugin_search: search,
        };
        routes::router(state)
    }

    fn sample_plugin(name: &str) -> Plugin {
        Plugin {
            id: PluginId::new(Uuid::new_v4()),
            name: name.into(),
            description: "Export project data".into(),
            category: "productivity".into(),
            tags: vec!["pdf".into(), "export".into()],
            price: 149_900,
            rating: Decimal::new(47, 1), // 4.7
            downloads: 12_500,
            status: PluginStatus::Published,
            updated_at: Utc::now().fixed_offset(),
        }
    }

    fn empty_page() -> Page<Plugin> {
        Page {
            items: vec![],
            total: 0,
            page: 1,
            limit: 20,
        }
    }

    async fn body_json(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }


    //Parse sort

    #[test]
    fn parse_sort_none_is_relevance() {
        assert!(matches!(parse_sort(&None).unwrap(), SortOrder::Relevance));
    }

    #[test]
    fn parse_sort_all_variants() {
        let cases = [
            ("relevance", SortOrder::Relevance),
            ("price_asc", SortOrder::PriceAsc),
            ("price_desc", SortOrder::PriceDesc),
            ("rating_desc", SortOrder::RatingDesc),
            ("downloads_desc", SortOrder::DownloadsDesc),
            ("newest", SortOrder::Newest),
        ];
        for (input, expected) in cases {
            let got = parse_sort(&Some(input.to_string())).unwrap();
            assert_eq!(got, expected, "input: {input}");
        }
    }

    #[test]
    fn parse_sort_unknown_returns_400() {
        let err = parse_sort(&Some("banana".into())).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("banana"));
    }

    //ApiError mapping

    #[test]
    fn api_error_from_not_found_is_404() {
        let e: ApiError = PluginError::NotFound(Uuid::nil()).into();
        assert_eq!(e.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn api_error_from_validation_is_400() {
        let e: ApiError = PluginError::Validation("bad".into()).into();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn api_error_from_stale_is_409() {
        let now = Utc::now().fixed_offset();
        let e: ApiError = PluginError::Stale {
            id: Uuid::nil(),
            incoming: now,
            existing: now,
        }
            .into();
        assert_eq!(e.status, StatusCode::CONFLICT);
    }

    #[test]
    fn api_error_from_conflict_is_409() {
        let e: ApiError = PluginError::Conflict(Uuid::nil()).into();
        assert_eq!(e.status, StatusCode::CONFLICT);
    }

    #[test]
    fn api_error_from_internal_hides_details() {
        let e: ApiError = PluginError::Internal(anyhow::anyhow!("db exploded")).into();
        assert_eq!(e.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(e.message, "internal server error");
        assert!(!e.message.contains("db exploded"));
    }

    #[test]
    fn api_error_validation_helper() {
        let e = ApiError::validation("nope");
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert_eq!(e.message, "nope");
    }

    //PluginResponse

    #[test]
    fn plugin_response_from_plugin() {
        let p = sample_plugin("Export Pro");
        let dto = PluginResponse::from(p);
        assert_eq!(dto.name, "Export Pro");
        assert!((dto.rating - 4.7).abs() < 1e-9);
        assert_eq!(dto.status, "published");
        assert_eq!(dto.price, 149_900);
        assert_eq!(dto.downloads, 12_500);
    }

    //HTTP

    #[tokio::test]
    async fn search_returns_200_with_items() {
        let plugin = sample_plugin("Export Pro");
        let page = Page {
            items: vec![plugin],
            total: 1,
            page: 1,
            limit: 20,
        };
        let mock = MockSearch::ok(page);
        let router = build_router(Arc::clone(&mock));

        let request = Request::builder()
            .uri("/api/v1/plugins/search?q=export")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let json = body_json(response).await;
        assert_eq!(json["total"], 1);
        assert_eq!(json["page"], 1);
        assert_eq!(json["limit"], 20);
        assert_eq!(json["total_pages"], 1);
        assert_eq!(json["items"][0]["name"], "Export Pro");
        assert_eq!(json["items"][0]["status"], "published");

        let received = mock.received().unwrap();
        assert_eq!(received.q.as_deref(), Some("export"));
    }

    #[tokio::test]
    async fn search_applies_defaults() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(Arc::clone(&mock));

        let request = Request::builder()
            .uri("/api/v1/plugins/search")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let received = mock.received().unwrap();
        assert_eq!(received.page, 1);
        assert_eq!(received.limit, 20);
        assert_eq!(received.sort, SortOrder::Relevance);
    }

    #[tokio::test]
    async fn search_clamps_limit_to_max() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(Arc::clone(&mock));

        let request = Request::builder()
            .uri("/api/v1/plugins/search?limit=9999")
            .body(Body::empty())
            .unwrap();
        router.oneshot(request).await.unwrap();

        let received = mock.received().unwrap();
        assert_eq!(received.limit, MAX_LIMIT);
    }
    //Работает только для разделителя запятой, нет поиска при множестве `?tags=abc&tags=zxc`
    #[tokio::test]
    async fn search_parses_tags_comma_separated() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(Arc::clone(&mock));

        let request = Request::builder()
            .uri("/api/v1/plugins/search?tags=pdf,export")
            .body(Body::empty())
            .unwrap();
        router.oneshot(request).await.unwrap();

        let received = mock.received().unwrap();
        assert_eq!(received.tags, vec!["pdf".to_string(), "export".to_string()]);
    }


    #[tokio::test]
    async fn search_parses_min_rating_as_decimal() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(Arc::clone(&mock));

        let request = Request::builder()
            .uri("/api/v1/plugins/search?min_rating=4.5")
            .body(Body::empty())
            .unwrap();
        router.oneshot(request).await.unwrap();

        let received = mock.received().unwrap();
        assert_eq!(received.min_rating, Some(Decimal::new(45, 1)));
    }

    //HTTP ошибки

    #[tokio::test]
    async fn search_unknown_sort_returns_400() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/api/v1/plugins/search?sort=banana")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let json = body_json(response).await;
        assert!(json["error"].as_str().unwrap().contains("banana"));
    }

    #[tokio::test]
    async fn search_invalid_query_params_returns_400() {
        // `Query<SearchQuery>` отвергнет неконвертируемые параметры
        let mock = MockSearch::ok(empty_page());
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/api/v1/plugins/search?page=abc")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_validation_error_from_service_returns_400() {
        let mock = MockSearch::err(PluginError::Validation("page must be >= 1".into()));
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/api/v1/plugins/search?page=0")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn search_internal_error_returns_500_without_details() {
        let mock = MockSearch::err(PluginError::Internal(anyhow::anyhow!("db exploded")));
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/api/v1/plugins/search")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let json = body_json(response).await;
        assert_eq!(json["error"], "internal server error");
        assert!(!json["error"].as_str().unwrap().contains("db exploded"));
    }

    //HTTP роутинг

    #[tokio::test]
    async fn health_returns_ok() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"ok");
    }

    #[tokio::test]
    async fn unknown_route_returns_404() {
        let mock = MockSearch::ok(empty_page());
        let router = build_router(mock);

        let request = Request::builder()
            .uri("/api/v1/nope")
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
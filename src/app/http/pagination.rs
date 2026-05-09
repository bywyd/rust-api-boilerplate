use serde::{Deserialize, Serialize};

const DEFAULT_PER_PAGE: u64 = 20;
const MAX_PER_PAGE: u64 = 100;

/// Query parameters for paginated list endpoints.
///
/// Bind automatically from the request URL via `web::Query<PaginationParams>`.
/// Example: `GET /api/users?page=2&per_page=20`
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Current page, 1-based. Defaults to `1`.
    pub page: Option<u64>,
    /// Number of items per page. Clamped to `1..=100`. Defaults to `20`.
    pub per_page: Option<u64>,
}

impl PaginationParams {
    /// Returns the resolved page number (minimum 1).
    pub fn page(&self) -> u64 {
        self.page.unwrap_or(1).max(1)
    }

    /// Returns the resolved page size, clamped between 1 and `MAX_PER_PAGE`.
    pub fn per_page(&self) -> u64 {
        self.per_page
            .unwrap_or(DEFAULT_PER_PAGE)
            .clamp(1, MAX_PER_PAGE)
    }

    /// Returns the SQL `OFFSET` for this page.
    pub fn offset(&self) -> u64 {
        (self.page() - 1) * self.per_page()
    }
}

/// Standard paginated response envelope returned by all list endpoints.
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub data: Vec<T>,
    /// Total number of matching records across all pages.
    pub total: u64,
    /// Current page (1-based).
    pub page: u64,
    /// Items per page as returned (after clamping).
    pub per_page: u64,
    /// Total number of pages.
    pub total_pages: u64,
}

impl<T: Serialize> PaginatedResponse<T> {
    pub fn new(data: Vec<T>, total: u64, params: &PaginationParams) -> Self {
        let per_page = params.per_page();
        let total_pages = if per_page == 0 {
            0
        } else {
            total.div_ceil(per_page)
        };
        Self {
            data,
            total,
            page: params.page(),
            per_page,
            total_pages,
        }
    }
}

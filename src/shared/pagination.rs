//! Offset pagination shared by every list endpoint. No list is unbounded:
//! `per_page` is clamped to `MAX_PER_PAGE`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const DEFAULT_PER_PAGE: u32 = 25;
pub const MAX_PER_PAGE: u32 = 100;

/// Resolved, clamped page request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub page: u32,
    pub per_page: u32,
}

impl Page {
    pub fn new(page: Option<u32>, per_page: Option<u32>) -> Self {
        Self { page: page.unwrap_or(1).max(1), per_page: per_page.unwrap_or(DEFAULT_PER_PAGE).clamp(1, MAX_PER_PAGE) }
    }

    /// First page with an explicit size (internal callers such as the dashboard).
    pub fn first(per_page: u32) -> Self {
        Self::new(Some(1), Some(per_page))
    }

    pub fn limit(self) -> i64 {
        i64::from(self.per_page)
    }

    pub fn offset(self) -> i64 {
        i64::from(self.page - 1) * i64::from(self.per_page)
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
pub struct PageMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: i64,
    pub total_pages: i64,
}

impl PageMeta {
    pub fn new(page: Page, total: i64) -> Self {
        let per_page = i64::from(page.per_page);
        Self { page: page.page, per_page: page.per_page, total, total_pages: (total + per_page - 1) / per_page }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    #[default]
    Desc,
}

impl SortOrder {
    /// SQL keyword. A fixed string, never user input.
    pub fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_clamping() {
        assert_eq!(Page::new(None, None), Page { page: 1, per_page: DEFAULT_PER_PAGE });
        assert_eq!(Page::new(Some(0), Some(0)), Page { page: 1, per_page: 1 });
        assert_eq!(Page::new(Some(3), Some(10_000)), Page { page: 3, per_page: MAX_PER_PAGE });
    }

    #[test]
    fn offset_and_total_pages() {
        let page = Page::new(Some(3), Some(20));
        assert_eq!((page.limit(), page.offset()), (20, 40));
        assert_eq!(PageMeta::new(page, 0).total_pages, 0);
        assert_eq!(PageMeta::new(page, 20).total_pages, 1);
        assert_eq!(PageMeta::new(page, 21).total_pages, 2);
    }

    #[test]
    fn huge_page_numbers_do_not_overflow() {
        let page = Page::new(Some(u32::MAX), Some(MAX_PER_PAGE));
        assert!(page.offset() > 0);
    }
}

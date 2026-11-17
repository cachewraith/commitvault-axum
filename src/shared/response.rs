//! Response envelopes. Every successful JSON response is either `Data<T>` or
//! `Paginated<T>`; errors use `errors::ErrorResponse`.

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use super::pagination::{Page, PageMeta};

#[derive(Debug, Serialize, ToSchema)]
pub struct Data<T> {
    pub data: T,
}

impl<T> Data<T> {
    pub fn json(data: T) -> Json<Self> {
        Json(Self { data })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Paginated<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}

impl<T> Paginated<T> {
    pub fn new(data: Vec<T>, page: Page, total: i64) -> Self {
        Self { data, meta: PageMeta::new(page, total) }
    }

    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Paginated<U> {
        Paginated { data: self.data.into_iter().map(f).collect(), meta: self.meta }
    }
}

use serde::{Deserialize, Serialize};

// Search related structs
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub keywords: Option<String>,
    pub has_location: Option<bool>,
    pub country: Option<String>,
    pub limit: Option<u32>,
    pub page: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchSuggestion {
    pub term: String,
    pub count: i64,
    pub category: String,
}

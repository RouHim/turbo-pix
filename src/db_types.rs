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

// Timeline related structs
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TimelineDensity {
    pub year: i32,
    pub month: i32,
    pub count: i64,
}

#[derive(Debug, Serialize)]
pub struct TimelineData {
    pub min_date: Option<String>,
    pub max_date: Option<String>,
    pub density: Vec<TimelineDensity>,
}

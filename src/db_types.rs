use serde::{Deserialize, Serialize};

// Search related structs
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
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

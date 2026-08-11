use serde::{Deserialize, Serialize};

// Search related structs
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
}

// Timeline related structs
#[derive(Debug, Serialize, Deserialize, PartialEq, sqlx::FromRow)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_data_serializes_with_expected_shape() {
        let data = TimelineData {
            min_date: Some("2024-01-01".to_string()),
            max_date: Some("2024-12-31".to_string()),
            density: vec![TimelineDensity {
                year: 2024,
                month: 1,
                count: 5,
            }],
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["min_date"], "2024-01-01");
        assert_eq!(json["max_date"], "2024-12-31");
        assert_eq!(json["density"][0]["count"], 5);
    }

    #[test]
    fn search_query_deserializes_optional_fields() {
        let q: SearchQuery = serde_json::from_str(r#"{"q":"cat","year":2024}"#).unwrap();
        assert_eq!(q.q.as_deref(), Some("cat"));
        assert_eq!(q.year, Some(2024));
        assert_eq!(q.month, None);
        let empty: SearchQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.q, None);
        assert_eq!(empty.year, None);
        assert_eq!(empty.month, None);
    }
}

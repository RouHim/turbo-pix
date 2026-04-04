use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use serde_json::Value;

use crate::db;

const CACHE_CAP: usize = 100_000;
const USER_AGENT: &str = "TurboPix/0.1 (photo-gallery)";

pub struct NominatimClient {
    base_url: String,
    agent: ureq::Agent,
    coordinate_cache: HashMap<(i64, i64), Option<String>>,
}

impl NominatimClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new().user_agent(USER_AGENT).build(),
            coordinate_cache: HashMap::new(),
        }
    }

    pub fn resolve_city_name(
        &mut self,
        lat: f64,
        lon: f64,
    ) -> Result<Option<String>, Box<dyn Error>> {
        let cache_key = Self::cache_key(lat, lon);

        if let Some(cached_city) = self.coordinate_cache.get(&cache_key) {
            return Ok(cached_city.clone());
        }

        let request_result = self
            .agent
            .get(&format!(
                "{}/reverse?lat={lat}&lon={lon}&format=json&zoom=10",
                self.base_url
            ))
            .call();

        std::thread::sleep(Duration::from_secs(1));

        let city = match request_result {
            Ok(response) => {
                let response_body = response.into_string()?;
                parse_city_from_response(&response_body)
            }
            Err(ureq::Error::Status(404, _response)) => None,
            Err(error) => return Err(Box::new(error)),
        };

        self.insert_cache_entry(cache_key, city.clone());

        Ok(city)
    }

    pub async fn resolve_batch<F>(
        &mut self,
        photos: Vec<(String, f64, f64)>,
        pool: &sqlx::SqlitePool,
        progress_cb: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: Fn(usize, usize),
    {
        let total = photos.len();

        for (index, (file_path, lat, lon)) in photos.into_iter().enumerate() {
            let city = self.resolve_city_name(lat, lon)?;
            db::update_photo_city(pool, &file_path, city.as_deref()).await?;
            progress_cb(index + 1, total);
        }

        Ok(())
    }

    fn cache_key(lat: f64, lon: f64) -> (i64, i64) {
        ((lat * 1000.0) as i64, (lon * 1000.0) as i64)
    }

    fn insert_cache_entry(&mut self, key: (i64, i64), value: Option<String>) {
        if self.coordinate_cache.len() >= CACHE_CAP {
            self.coordinate_cache.clear();
        }

        self.coordinate_cache.insert(key, value);
    }
}

fn parse_city_from_response(json_str: &str) -> Option<String> {
    let response_json: Value = serde_json::from_str(json_str).ok()?;
    let address = response_json.get("address")?;

    ["city", "town", "village", "hamlet", "municipality"]
        .iter()
        .find_map(|field_name| {
            address
                .get(field_name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_cache_rounding() {
        let first_key = NominatimClient::cache_key(52.5201, 13.4051);
        let second_key = NominatimClient::cache_key(52.5209, 13.4059);

        assert_eq!(first_key, (52_520, 13_405));
        assert_eq!(first_key, second_key);
    }

    #[test]
    fn test_coordinate_cache_different() {
        let first_key = NominatimClient::cache_key(52.520, 13.405);
        let second_key = NominatimClient::cache_key(52.530, 13.405);

        assert_ne!(first_key, second_key);
    }

    #[test]
    fn test_parse_nominatim_city() {
        let json = r#"{"address":{"city":"Berlin"}}"#;

        assert_eq!(parse_city_from_response(json), Some("Berlin".to_string()));
    }

    #[test]
    fn test_parse_nominatim_town_fallback() {
        let json = r#"{"address":{"town":"Kleinstadt"}}"#;

        assert_eq!(
            parse_city_from_response(json),
            Some("Kleinstadt".to_string())
        );
    }

    #[test]
    fn test_parse_nominatim_village_fallback() {
        let json = r#"{"address":{"village":"Dorf"}}"#;

        assert_eq!(parse_city_from_response(json), Some("Dorf".to_string()));
    }

    #[test]
    fn test_parse_nominatim_hamlet_fallback() {
        let json = r#"{"address":{"hamlet":"Weiler"}}"#;

        assert_eq!(parse_city_from_response(json), Some("Weiler".to_string()));
    }

    #[test]
    fn test_parse_nominatim_municipality_fallback() {
        let json = r#"{"address":{"municipality":"Gemeinde"}}"#;

        assert_eq!(parse_city_from_response(json), Some("Gemeinde".to_string()));
    }

    #[test]
    fn test_parse_nominatim_no_address() {
        let json = r#"{"address":{}}"#;

        assert_eq!(parse_city_from_response(json), None);
    }

    #[test]
    fn test_cache_cap_enforcement() {
        let mut client = NominatimClient::new("https://nominatim.openstreetmap.org");

        for index in 0..=CACHE_CAP {
            client.insert_cache_entry((index as i64, index as i64), Some(index.to_string()));
        }

        assert!(client.coordinate_cache.len() <= CACHE_CAP);
    }

    #[test]
    fn test_address_priority_order() {
        let json = r#"{"address":{"city":"Berlin","town":"Kleinstadt"}}"#;

        assert_eq!(parse_city_from_response(json), Some("Berlin".to_string()));
    }
}

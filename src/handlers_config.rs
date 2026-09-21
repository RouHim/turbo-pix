use serde::Serialize;
use warp::Filter;

#[derive(Serialize)]
struct ConfigResponse {
    default_locale: String,
    tile_url: String,
}

pub fn build_config_routes(
    default_locale: String,
    tile_url: String,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "config").and(warp::get()).map(move || {
        warp::reply::json(&ConfigResponse {
            default_locale: default_locale.clone(),
            tile_url: tile_url.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_route_returns_default_locale_and_tile_url() {
        let routes = build_config_routes(
            "de".to_string(),
            "http://tiles.lan/{z}/{x}/{y}.png".to_string(),
        );
        let res = warp::test::request()
            .path("/api/config")
            .reply(&routes)
            .await;
        assert_eq!(res.status(), 200);
        let body: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
        assert_eq!(body["default_locale"], "de");
        assert_eq!(body["tile_url"], "http://tiles.lan/{z}/{x}/{y}.png");
    }
}

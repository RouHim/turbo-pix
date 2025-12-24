use serde::Serialize;
use warp::Filter;

#[derive(Serialize)]
struct ConfigResponse {
    default_locale: String,
}

pub fn build_config_routes(
    default_locale: String,
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path!("api" / "config").and(warp::get()).map(move || {
        warp::reply::json(&ConfigResponse {
            default_locale: default_locale.clone(),
        })
    })
}

use warp::Filter;

macro_rules! include_static {
    ($($path:expr),* $(,)?) => {
        &[
            $(($path, include_str!(concat!("../static/", $path)))),*
        ]
    };
}

const STATIC_FILES: &[(&str, &str)] = include_static![
    "css/main.css",
    "css/components.css",
    "css/responsive.css",
    "js/utils.js",
    "js/logger.js",
    "js/api.js",
    "js/photoGrid.js",
    "js/viewer.js",
    "js/search.js",
    "js/timeline.js",
    "js/i18n.js",
    "js/app.js",
    "js/feather.min.js",
    "js/icons.js",
    "i18n/i18nManager.js",
    "i18n/en/index.js",
    "i18n/de/index.js",
];

fn content_type_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("html") => "text/html",
        _ => "text/plain",
    }
}

pub fn build_static_routes(
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let index_route = warp::path::end().and(warp::get()).map(|| {
        warp::reply::with_header(
            include_str!("../static/index.html"),
            "content-type",
            "text/html",
        )
    });

    let file_route = warp::path::full().and(warp::get()).and_then(
        |full_path: warp::path::FullPath| async move {
            let path = full_path.as_str().trim_start_matches('/');

            for (file_path, content) in STATIC_FILES {
                if *file_path == path {
                    let content_type = content_type_from_path(path);
                    return Ok::<_, warp::Rejection>(warp::reply::with_header(
                        *content,
                        "content-type",
                        content_type,
                    ));
                }
            }

            Err(warp::reject::not_found())
        },
    );

    index_route.or(file_route)
}

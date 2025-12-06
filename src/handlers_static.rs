use warp::Filter;

macro_rules! include_static {
    ($($path:expr),* $(,)?) => {
        &[$(($path, include_str!(concat!("../static/", $path)))),*]
    };
}

const STATIC_FILES: &[(&str, &str)] = include_static![
    "index.html",
    "favicon.svg",
    "site.webmanifest",
    "css/main.css",
    "css/components.css",
    "css/responsive.css",
    "js/constants.js",
    "js/utils.js",
    "js/logger.js",
    "js/blurhash.js",
    "js/api.js",
    "js/viewerControls.js",
    "js/viewerMetadata.js",
    "js/viewerMetadataEdit.js",
    "js/photoCard.js",
    "js/infiniteScroll.js",
    "js/photoGrid.js",
    "js/viewer.js",
    "js/search.js",
    "js/timeline.js",
    "js/i18n.js",
    "js/app.js",
    "js/collages.js",
    "js/indexingStatus.js",
    "js/gestureManager.js",
    "js/gestureRecognizers.js",
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
        Some("svg") => "image/svg+xml",
        _ => "text/plain",
    }
}

fn build_route_for_file(
    path: &'static str,
    content: &'static str,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    let segments: Vec<&str> = path.split('/').collect();
    let content_type = content_type_from_path(path);

    if segments.len() == 1 && segments[0] == "index.html" {
        return warp::path::end()
            .and(warp::get())
            .map(move || warp::reply::with_header(content, "content-type", content_type))
            .boxed();
    }

    let mut filter = warp::path(segments[0]).boxed();
    for segment in segments.iter().skip(1) {
        filter = filter.and(warp::path(*segment)).boxed();
    }

    filter
        .and(warp::path::end())
        .and(warp::get())
        .map(move || warp::reply::with_header(content, "content-type", content_type))
        .boxed()
}

pub fn build_static_routes(
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let mut iter = STATIC_FILES
        .iter()
        .map(|(path, content)| build_route_for_file(path, content));
    let first = iter
        .next()
        .expect("At least one static file must be defined");

    iter.fold(first.boxed(), |acc, route| acc.or(route).unify().boxed())
}

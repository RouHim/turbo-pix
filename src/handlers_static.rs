use warp::reply::Reply;
use warp::Filter;

include!(concat!(env!("OUT_DIR"), "/embedded_static.rs"));

fn content_type_from_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("html") => "text/html",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("webmanifest") => "application/manifest+json",
        Some("json") => "application/json",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "text/plain",
    }
}

/// Builds the response for an embedded asset. Vite emits unhashed filenames
/// (index.js/index.css), so the browser must revalidate on every load and a
/// frontend update cannot be served stale from a heuristic cache.
fn build_asset_response(
    content: &'static [u8],
    content_type: &'static str,
) -> warp::reply::Response {
    warp::reply::with_header(
        warp::reply::with_header(content.to_vec(), "content-type", content_type),
        "cache-control",
        "no-cache",
    )
    .into_response()
}

/// Builds the HEAD mirror of an asset route: the same headers as the GET
/// response (content-type, content-length, cache-control) with an empty body.
/// Notably no `accept-ranges` header: the static GET route ignores Range, so
/// advertising byte-range support would be wrong.
fn build_head_response(
    content: &'static [u8],
    content_type: &'static str,
) -> warp::reply::Response {
    warp::reply::with_header(
        warp::reply::with_header(
            warp::reply::with_header(Vec::<u8>::new(), "content-type", content_type),
            "content-length",
            content.len().to_string(),
        ),
        "cache-control",
        "no-cache",
    )
    .into_response()
}

fn build_route(
    path: &'static str,
    content: &'static [u8],
) -> warp::filters::BoxedFilter<(warp::reply::Response,)> {
    let segments: Vec<&str> = path.split('/').collect();
    let content_type = content_type_from_path(path);

    if segments.len() == 1 && segments[0] == "index.html" {
        let get = warp::path::end()
            .and(warp::get())
            .map(move || build_asset_response(content, content_type));
        let head = warp::path::end()
            .and(warp::head())
            .map(move || build_head_response(content, content_type));
        return get.or(head).unify().boxed();
    }

    let mut filter = warp::path(segments[0]).boxed();
    for segment in segments.iter().skip(1) {
        filter = filter.and(warp::path(*segment)).boxed();
    }

    let get = filter
        .clone()
        .and(warp::path::end())
        .and(warp::get())
        .map(move || build_asset_response(content, content_type));
    let head = filter
        .and(warp::path::end())
        .and(warp::head())
        .map(move || build_head_response(content, content_type));

    get.or(head).unify().boxed()
}

pub fn build_static_routes(
) -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    let mut iter = STATIC_FILES
        .iter()
        .map(|(path, content)| build_route(path, content.as_bytes()));
    let first = iter
        .next()
        .expect("At least one static file must be defined");

    let all_static = iter.fold(first.boxed(), |acc, route| acc.or(route).unify().boxed());

    let all_binary_opt = STATIC_BINARY_FILES
        .iter()
        .map(|(path, content)| build_route(path, content))
        .fold(
            None::<warp::filters::BoxedFilter<(warp::reply::Response,)>>,
            |acc, route| {
                Some(match acc {
                    None => route,
                    Some(a) => a.or(route).unify().boxed(),
                })
            },
        );

    // Add catch-all route for SPA routing - serves index.html for all non-static paths
    let index_html = STATIC_FILES
        .iter()
        .find(|(path, _)| *path == "index.html")
        .map(|(_, content)| *content)
        .expect("index.html must be in static files");

    let spa_fallback = warp::get().and(warp::path::full()).and_then(
        move |path: warp::path::FullPath| async move {
            let path_str = path.as_str();
            // Reject API and static asset paths - let them be handled by specific routes or return 404
            if path_str.starts_with("/api/")
                || path_str.starts_with("/assets/")
                || path_str.starts_with("/favicon")
                || path_str.starts_with("/site.webmanifest")
                || path_str.starts_with("/fonts/")
            {
                Err(warp::reject::not_found())
            } else {
                // Serve index.html for all other GET requests (SPA routes)
                Ok::<_, warp::Rejection>(
                    warp::reply::with_header(
                        warp::reply::with_header(index_html, "content-type", "text/html"),
                        "cache-control",
                        "no-cache",
                    )
                    .into_response(),
                )
            }
        },
    );

    match all_binary_opt {
        Some(all_binary) => all_binary
            .or(all_static)
            .unify()
            .or(spa_fallback)
            .unify()
            .boxed(),
        None => all_static.or(spa_fallback).unify().boxed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_routes_serve_index_assets_and_spa_fallback() {
        let routes = build_static_routes();

        // index.html is served at the root with a no-cache header
        let index = warp::test::request().path("/").reply(&routes).await;
        assert_eq!(index.status(), 200);
        assert!(index.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert_eq!(index.headers()["cache-control"], "no-cache");

        // Every embedded asset is served at its path
        for (path, content) in STATIC_FILES
            .iter()
            .map(|(p, c)| (*p, c.as_bytes()))
            .chain(STATIC_BINARY_FILES.iter().map(|(p, c)| (*p, *c)))
        {
            let response = warp::test::request()
                .path(&format!("/{path}"))
                .reply(&routes)
                .await;
            assert_eq!(response.status(), 200, "asset {path} should be served");
            assert_eq!(response.body(), content, "asset {path} body mismatch");
            assert_eq!(response.headers()["cache-control"], "no-cache");
        }

        // HEAD requests on assets mirror the GET headers with an empty body
        let (head_path, head_content) = STATIC_FILES
            .iter()
            .map(|(p, c)| (*p, c.as_bytes()))
            .chain(STATIC_BINARY_FILES.iter().map(|(p, c)| (*p, *c)))
            .find(|(p, _)| *p != "index.html")
            .expect("at least one non-index asset is embedded");
        let head = warp::test::request()
            .method("HEAD")
            .path(&format!("/{head_path}"))
            .reply(&routes)
            .await;
        assert_eq!(head.status(), 200, "HEAD {head_path} should succeed");
        assert!(head.body().is_empty(), "HEAD body must be empty");
        assert_eq!(
            head.headers()["content-length"],
            head_content.len().to_string().as_str()
        );
        assert_eq!(
            head.headers()["content-type"],
            content_type_from_path(head_path)
        );
        assert!(
            head.headers().get("accept-ranges").is_none(),
            "static assets do not support range requests; HEAD must not advertise accept-ranges"
        );
        assert_eq!(head.headers()["cache-control"], "no-cache");

        // HEAD on unknown paths stays rejected (no SPA-fallback mirror)
        let head_unknown = warp::test::request()
            .method("HEAD")
            .path("/some/client/route")
            .reply(&routes)
            .await;
        assert_ne!(head_unknown.status(), 200);

        // Unknown paths fall back to index.html (SPA routing)
        let spa = warp::test::request()
            .path("/some/client/route")
            .reply(&routes)
            .await;
        assert_eq!(spa.status(), 200);
        assert!(spa.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));

        // API paths are rejected, not swallowed by the SPA fallback
        let api = warp::test::request()
            .path("/api/photos")
            .reply(&routes)
            .await;
        assert_eq!(api.status(), 404);
    }

    #[test]
    fn content_type_covers_embedded_assets() {
        assert_eq!(
            content_type_from_path("site.webmanifest"),
            "application/manifest+json"
        );
        assert_eq!(content_type_from_path("app.js"), "application/javascript");
        assert_eq!(content_type_from_path("favicon.svg"), "image/svg+xml");
        assert_eq!(content_type_from_path("index.css"), "text/css");
        assert_eq!(content_type_from_path("font.woff2"), "font/woff2");
        assert_eq!(content_type_from_path("data.json"), "application/json");
        assert_eq!(content_type_from_path("image.png"), "image/png");
        assert_eq!(content_type_from_path("favicon.ico"), "image/x-icon");
        assert_eq!(content_type_from_path("photo.webp"), "image/webp");
    }
}

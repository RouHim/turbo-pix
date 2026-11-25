use sha2::{Digest, Sha256};
use warp::reply::Reply;
use warp::Filter;

include!(concat!(env!("OUT_DIR"), "/embedded_static.rs"));

/// Strong validator for an embedded asset: the bytes only change with a new
/// build, so their digest is a stable representation tag. Vite emits unhashed
/// filenames, which is why the tag lives on the response instead of the URL.
fn asset_etag(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("\"{hex}\"")
}

/// `If-None-Match` compares weakly, so `W/"x"` matches `"x"` for this GET;
/// `*` matches any current representation.
fn etag_matches(if_none_match: Option<&str>, etag: &str) -> bool {
    if_none_match.is_some_and(|value| {
        value.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || candidate.trim_start_matches("W/") == etag
        })
    })
}

fn not_modified(etag: &str) -> warp::reply::Response {
    warp::reply::with_header(
        warp::reply::with_header(
            warp::reply::with_status(warp::reply(), warp::http::StatusCode::NOT_MODIFIED),
            "etag",
            etag.to_string(),
        ),
        "cache-control",
        "no-cache",
    )
    .into_response()
}

/// Builds the response for an embedded asset. Vite emits unhashed filenames
/// (index.js/index.css), so the browser must revalidate on every load and a
/// frontend update cannot be served stale from a heuristic cache; the ETag
/// turns that revalidation into a bodyless 304.
fn build_asset_response(
    content: &'static [u8],
    content_type: &'static str,
    etag: &str,
    if_none_match: Option<&str>,
) -> warp::reply::Response {
    if etag_matches(if_none_match, etag) {
        return not_modified(etag);
    }
    warp::reply::with_header(
        warp::reply::with_header(
            warp::reply::with_header(content.to_vec(), "content-type", content_type),
            "cache-control",
            "no-cache",
        ),
        "etag",
        etag.to_string(),
    )
    .into_response()
}

/// Builds the HEAD mirror of an asset route: the same headers as the GET
/// response (content-type, content-length, cache-control, etag) with an empty
/// body. Notably no `accept-ranges` header: the static GET route ignores Range,
/// so advertising byte-range support would be wrong.
fn build_head_response(
    content: &'static [u8],
    content_type: &'static str,
    etag: &str,
    if_none_match: Option<&str>,
) -> warp::reply::Response {
    if etag_matches(if_none_match, etag) {
        return not_modified(etag);
    }
    warp::reply::with_header(
        warp::reply::with_header(
            warp::reply::with_header(
                warp::reply::with_header(Vec::<u8>::new(), "content-type", content_type),
                "content-length",
                content.len().to_string(),
            ),
            "cache-control",
            "no-cache",
        ),
        "etag",
        etag.to_string(),
    )
    .into_response()
}

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

fn build_route(
    path: &'static str,
    content: &'static [u8],
) -> warp::filters::BoxedFilter<(warp::reply::Response,)> {
    let segments: Vec<&str> = path.split('/').collect();
    let content_type = content_type_from_path(path);
    let etag = asset_etag(content);
    let get_etag = etag.clone();

    if segments.len() == 1 && segments[0] == "index.html" {
        let get = warp::path::end()
            .and(warp::get())
            .and(warp::header::optional::<String>("if-none-match"))
            .map(move |if_none_match: Option<String>| {
                build_asset_response(content, content_type, &get_etag, if_none_match.as_deref())
            });
        let head = warp::path::end()
            .and(warp::head())
            .and(warp::header::optional::<String>("if-none-match"))
            .map(move |if_none_match: Option<String>| {
                build_head_response(content, content_type, &etag, if_none_match.as_deref())
            });
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
        .and(warp::header::optional::<String>("if-none-match"))
        .map(move |if_none_match: Option<String>| {
            build_asset_response(content, content_type, &get_etag, if_none_match.as_deref())
        });
    let head = filter
        .and(warp::path::end())
        .and(warp::head())
        .and(warp::header::optional::<String>("if-none-match"))
        .map(move |if_none_match: Option<String>| {
            build_head_response(content, content_type, &etag, if_none_match.as_deref())
        });

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
    let index_etag = asset_etag(index_html.as_bytes());

    let spa_fallback = warp::get()
        .and(warp::path::full())
        .and(warp::header::optional::<String>("if-none-match"))
        .and_then(
            move |path: warp::path::FullPath, if_none_match: Option<String>| {
                let index_etag = index_etag.clone();
                async move {
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
                        Ok::<_, warp::Rejection>(build_asset_response(
                            index_html.as_bytes(),
                            "text/html",
                            &index_etag,
                            if_none_match.as_deref(),
                        ))
                    }
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

    #[tokio::test]
    async fn static_assets_serve_a_strong_etag_and_revalidate() {
        let routes = build_static_routes();
        let (path, content) = STATIC_FILES
            .iter()
            .find(|(p, _)| *p != "index.html")
            .map(|(p, c)| (*p, c.as_bytes()))
            .expect("at least one non-index text asset");

        let first = warp::test::request()
            .path(&format!("/{path}"))
            .reply(&routes)
            .await;
        assert_eq!(first.status(), 200);
        assert_eq!(first.body(), content);
        let etag = first.headers()["etag"]
            .to_str()
            .expect("assets carry an ETag")
            .to_string();
        assert!(
            etag.starts_with('"') && etag.ends_with('"'),
            "ETag must be a quoted strong validator, got {etag}"
        );

        let revalidated = warp::test::request()
            .path(&format!("/{path}"))
            .header("if-none-match", &etag)
            .reply(&routes)
            .await;
        assert_eq!(revalidated.status(), 304);
        assert!(revalidated.body().is_empty(), "304 must not carry a body");
        assert_eq!(revalidated.headers()["etag"], etag);
        assert_eq!(revalidated.headers()["cache-control"], "no-cache");

        let stale = warp::test::request()
            .path(&format!("/{path}"))
            .header("if-none-match", "\"stale\"")
            .reply(&routes)
            .await;
        assert_eq!(stale.status(), 200, "a different validator gets the body");
        assert_eq!(stale.body(), content);

        let head = warp::test::request()
            .method("HEAD")
            .path(&format!("/{path}"))
            .reply(&routes)
            .await;
        assert_eq!(head.headers()["etag"], etag, "HEAD mirrors the ETag");

        // The SPA fallback serves index.html with its own validator.
        let spa = warp::test::request()
            .path("/client/route")
            .reply(&routes)
            .await;
        assert_eq!(spa.status(), 200);
        let index_etag = spa.headers()["etag"]
            .to_str()
            .expect("the SPA fallback carries an ETag")
            .to_string();
        let spa_revalidated = warp::test::request()
            .path("/client/route")
            .header("if-none-match", &index_etag)
            .reply(&routes)
            .await;
        assert_eq!(spa_revalidated.status(), 304);
        assert!(spa_revalidated.body().is_empty());
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

use actix_web::{test, web, App};
use turbo_pix::web::static_handler::{get_mime_type, serve_static_asset, StaticAsset};

#[actix_web::test]
async fn test_serve_index_html() {
    let app = test::init_service(App::new().route("/", web::get().to(serve_static_asset))).await;

    let req = test::TestRequest::get().uri("/").to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/html; charset=utf-8"
    );

    let body = test::read_body(resp).await;
    let body_str = std::str::from_utf8(&body).unwrap();

    assert!(body_str.contains("<!DOCTYPE html>"));
    assert!(body_str.contains("<title>TurboPix</title>"));
    assert!(body_str.contains("id=\"app\""));
}

#[actix_web::test]
async fn test_serve_css_files() {
    let app =
        test::init_service(App::new().route("/css/{filename}", web::get().to(serve_static_asset)))
            .await;

    let req = test::TestRequest::get().uri("/css/main.css").to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/css; charset=utf-8"
    );
}

#[actix_web::test]
async fn test_serve_js_files() {
    let app =
        test::init_service(App::new().route("/js/{filename}", web::get().to(serve_static_asset)))
            .await;

    let req = test::TestRequest::get().uri("/js/app.js").to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/javascript; charset=utf-8"
    );
}

#[actix_web::test]
async fn test_serve_nonexistent_file() {
    let app =
        test::init_service(App::new().route("/{path:.*}", web::get().to(serve_static_asset))).await;

    let req = test::TestRequest::get()
        .uri("/nonexistent.css")
        .to_request();

    let resp = test::call_service(&app, req).await;

    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn test_get_mime_type() {
    assert_eq!(get_mime_type("test.html"), "text/html; charset=utf-8");
    assert_eq!(get_mime_type("style.css"), "text/css; charset=utf-8");
    assert_eq!(
        get_mime_type("script.js"),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(get_mime_type("image.png"), "image/png");
    assert_eq!(get_mime_type("image.jpg"), "image/jpeg");
    assert_eq!(get_mime_type("image.jpeg"), "image/jpeg");
    assert_eq!(get_mime_type("image.gif"), "image/gif");
    assert_eq!(get_mime_type("image.svg"), "image/svg+xml");
    assert_eq!(get_mime_type("unknown"), "application/octet-stream");
}

#[actix_web::test]
async fn test_static_asset_enum() {
    let asset = StaticAsset::from_path("/");
    assert!(matches!(asset, StaticAsset::IndexHtml));

    let asset = StaticAsset::from_path("/index.html");
    assert!(matches!(asset, StaticAsset::IndexHtml));

    let asset = StaticAsset::from_path("/css/main.css");
    assert!(matches!(asset, StaticAsset::MainCss));

    let asset = StaticAsset::from_path("/js/app.js");
    assert!(matches!(asset, StaticAsset::AppJs));

    let asset = StaticAsset::from_path("/nonexistent.txt");
    assert!(matches!(asset, StaticAsset::NotFound));
}

#[actix_web::test]
async fn test_static_asset_content() {
    let asset = StaticAsset::IndexHtml;
    let content = asset.content();
    assert!(content.is_some());
    assert!(content.unwrap().contains("<!DOCTYPE html>"));

    let asset = StaticAsset::MainCss;
    let content = asset.content();
    assert!(content.is_some());

    let asset = StaticAsset::AppJs;
    let content = asset.content();
    assert!(content.is_some());

    let asset = StaticAsset::NotFound;
    let content = asset.content();
    assert!(content.is_none());
}

#[actix_web::test]
async fn test_static_asset_mime_type() {
    assert_eq!(
        StaticAsset::IndexHtml.mime_type(),
        "text/html; charset=utf-8"
    );
    assert_eq!(StaticAsset::MainCss.mime_type(), "text/css; charset=utf-8");
    assert_eq!(
        StaticAsset::AppJs.mime_type(),
        "application/javascript; charset=utf-8"
    );
}

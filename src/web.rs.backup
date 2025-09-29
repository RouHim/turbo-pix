// Flattened web module containing: middleware, routes, static_handler, and api_tests

// ============================================================================
// MIDDLEWARE MODULE (from src/web/middleware.rs)
// ============================================================================

pub mod middleware {
    use actix_web::{http::header, middleware::DefaultHeaders, middleware::Logger};

    #[allow(dead_code)]
    pub fn configure_middleware() -> Logger {
        Logger::default()
    }

    pub fn cors_headers() -> DefaultHeaders {
        DefaultHeaders::new()
            .add((header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"))
            .add((
                header::ACCESS_CONTROL_ALLOW_METHODS,
                "GET, POST, PUT, DELETE, OPTIONS",
            ))
            .add((
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type, Authorization",
            ))
    }
}

// ============================================================================
// ROUTES MODULE (from src/web/routes.rs)
// ============================================================================

pub mod routes {
    use crate::web_handlers;
    use actix_web::web;

    pub fn configure_routes(cfg: &mut web::ServiceConfig) {
        cfg.route("/health", web::get().to(web_handlers::health_check))
            .route("/ready", web::get().to(web_handlers::ready_check))
            .service(
                web::scope("/api")
                    .route("/photos", web::get().to(web_handlers::list_photos))
                    .route("/photos", web::post().to(web_handlers::upload_photo))
                    .route(
                        "/photos/{id}/file",
                        web::get().to(web_handlers::get_photo_file),
                    )
                    .route(
                        "/photos/{id}/video",
                        web::get().to(web_handlers::get_video_file),
                    )
                    .route(
                        "/photos/{id}/metadata",
                        web::get().to(web_handlers::get_photo_metadata),
                    )
                    .route("/photos/{id}", web::get().to(web_handlers::get_photo))
                    .route("/photos/{id}", web::put().to(web_handlers::update_photo))
                    .route("/photos/{id}", web::delete().to(web_handlers::delete_photo))
                    .route(
                        "/photos/{id}/favorite",
                        web::put().to(web_handlers::toggle_photo_favorite),
                    )
                    .route("/search", web::get().to(web_handlers::search_photos))
                    .route(
                        "/search/suggestions",
                        web::get().to(web_handlers::search_suggestions),
                    )
                    .route("/collections", web::get().to(web_handlers::get_collections))
                    .route("/cameras", web::get().to(web_handlers::get_cameras))
                    .route("/stats", web::get().to(web_handlers::get_stats))
                    .route(
                        "/thumbnails/hash/{hash}",
                        web::get().to(web_handlers::get_thumbnail_by_hash_default_size),
                    )
                    .route(
                        "/thumbnails/hash/{hash}/{size}",
                        web::get().to(web_handlers::get_thumbnail_by_hash),
                    )
                    .route("/cache/stats", web::get().to(web_handlers::cache_stats))
                    .route("/cache/clear", web::delete().to(web_handlers::clear_cache)),
            );
    }
}

// ============================================================================
// STATIC HANDLER MODULE (from src/web/static_handler.rs)
// ============================================================================

pub mod static_handler {
    use actix_web::{HttpRequest, HttpResponse, Result};

    #[derive(Debug, Clone)]
    pub enum StaticAsset {
        IndexHtml,
        MainCss,
        ComponentsCss,
        ResponsiveCss,
        AppJs,
        ApiJs,
        LoggerJs,
        PhotoGridJs,
        ViewerJs,
        SearchJs,
        UtilsJs,
        I18nManagerJs,
        I18nEnIndexJs,
        I18nDeIndexJs,
        NotFound,
    }

    impl StaticAsset {
        pub fn from_path(path: &str) -> Self {
            match path {
                "/" | "/index.html" => Self::IndexHtml,
                "/css/main.css" => Self::MainCss,
                "/css/components.css" => Self::ComponentsCss,
                "/css/responsive.css" => Self::ResponsiveCss,
                "/js/app.js" => Self::AppJs,
                "/js/api.js" => Self::ApiJs,
                "/js/logger.js" => Self::LoggerJs,
                "/js/photoGrid.js" => Self::PhotoGridJs,
                "/js/viewer.js" => Self::ViewerJs,
                "/js/search.js" => Self::SearchJs,
                "/js/utils.js" => Self::UtilsJs,
                "/i18n/i18nManager.js" => Self::I18nManagerJs,
                "/i18n/en/index.js" => Self::I18nEnIndexJs,
                "/i18n/de/index.js" => Self::I18nDeIndexJs,
                _ => Self::NotFound,
            }
        }

        pub fn content(&self) -> Option<&'static str> {
            match self {
                Self::IndexHtml => Some(include_str!("../static/index.html")),
                Self::MainCss => Some(include_str!("../static/css/main.css")),
                Self::ComponentsCss => Some(include_str!("../static/css/components.css")),
                Self::ResponsiveCss => Some(include_str!("../static/css/responsive.css")),
                Self::AppJs => Some(include_str!("../static/js/app.js")),
                Self::ApiJs => Some(include_str!("../static/js/api.js")),
                Self::LoggerJs => Some(include_str!("../static/js/logger.js")),
                Self::PhotoGridJs => Some(include_str!("../static/js/photoGrid.js")),
                Self::ViewerJs => Some(include_str!("../static/js/viewer.js")),
                Self::SearchJs => Some(include_str!("../static/js/search.js")),
                Self::UtilsJs => Some(include_str!("../static/js/utils.js")),
                Self::I18nManagerJs => Some(include_str!("../static/i18n/i18nManager.js")),
                Self::I18nEnIndexJs => Some(include_str!("../static/i18n/en/index.js")),
                Self::I18nDeIndexJs => Some(include_str!("../static/i18n/de/index.js")),
                Self::NotFound => None,
            }
        }

        pub fn mime_type(&self) -> &'static str {
            match self {
                Self::IndexHtml => "text/html; charset=utf-8",
                Self::MainCss | Self::ComponentsCss | Self::ResponsiveCss => {
                    "text/css; charset=utf-8"
                }
                Self::AppJs
                | Self::ApiJs
                | Self::LoggerJs
                | Self::PhotoGridJs
                | Self::ViewerJs
                | Self::SearchJs
                | Self::UtilsJs
                | Self::I18nManagerJs
                | Self::I18nEnIndexJs
                | Self::I18nDeIndexJs => "application/javascript; charset=utf-8",
                Self::NotFound => "text/plain",
            }
        }
    }

    #[allow(dead_code)]
    pub fn get_mime_type(filename: &str) -> &'static str {
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        match extension.to_lowercase().as_str() {
            "html" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" => "application/javascript; charset=utf-8",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        }
    }

    pub async fn serve_static_asset(req: HttpRequest) -> Result<HttpResponse> {
        let path = req.path();
        tracing::info!("Serving static asset for path: {}", path);
        let asset = StaticAsset::from_path(path);
        tracing::info!("Asset type: {:?}", asset);

        match asset.content() {
            Some(content) => Ok(HttpResponse::Ok()
                .content_type(asset.mime_type())
                .body(content)),
            None => {
                tracing::warn!("Asset not found for path: {}", path);
                Ok(HttpResponse::NotFound()
                    .content_type("text/plain")
                    .body("File not found"))
            }
        }
    }
}

// ============================================================================
// TESTS (from various modules)
// ============================================================================

#[cfg(test)]
mod static_handler_tests {
    pub mod tests {
        use super::super::static_handler::*;
        use actix_web::{test, web, App};

        #[actix_web::test]
        async fn test_serve_index_html() {
            let app =
                test::init_service(App::new().route("/", web::get().to(serve_static_asset))).await;

            let req = test::TestRequest::get().uri("/").to_request();

            let resp = test::call_service(&app, req).await;

            assert_eq!(resp.status(), 200);
            assert_eq!(
                resp.headers().get("content-type").unwrap(),
                "text/html; charset=utf-8"
            );

            let body = test::read_body(resp).await;
            let body_str = std::str::from_utf8(&body).unwrap();

            assert!(body_str.contains("<!doctype html>"));
            assert!(body_str.contains("<title>TurboPix</title>"));
            assert!(body_str.contains("id=\"app\""));
        }

        #[actix_web::test]
        async fn test_serve_css_files() {
            let app = test::init_service(
                App::new().route("/css/{filename}", web::get().to(serve_static_asset)),
            )
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
            let app = test::init_service(
                App::new().route("/js/{filename}", web::get().to(serve_static_asset)),
            )
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
            let app = test::init_service(
                App::new().route("/{path:.*}", web::get().to(serve_static_asset)),
            )
            .await;

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
            let html_content = content.unwrap();
            assert!(html_content.contains("<!doctype html>"));

            // Test for the info-toggle button
            assert!(
                html_content.contains("info-toggle"),
                "HTML should contain info-toggle button"
            );
            println!(
                "HTML content (first 500 chars): {}",
                &html_content[..500.min(html_content.len())]
            );

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
    }
}

#[cfg(test)]
pub mod api_tests {
    use crate::db::{create_in_memory_pool, DbPool, Photo};
    use crate::web::routes::configure_routes;
    use actix_web::{test, web, App};
    use bytes::Bytes;
    use serde_json::{json, Value};
    use std::io::Write;
    use tempfile::NamedTempFile;

    struct TestContext {
        pool: DbPool,
    }

    impl TestContext {
        fn new() -> Self {
            Self {
                pool: create_in_memory_pool().unwrap(),
            }
        }

        fn create_test_photo(&self, path: &str, filename: &str) -> i64 {
            let photo = Photo::new_test_photo(path.to_string(), filename.to_string());
            photo.create(&self.pool).unwrap()
        }

        fn create_test_jpeg() -> (NamedTempFile, Vec<u8>) {
            let mut temp_file = NamedTempFile::new().unwrap();
            // Simple JPEG header for testing
            let jpeg_data = vec![
                0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x01,
                0x00, 0x48, 0x00, 0x48, 0x00, 0x00, 0xFF, 0xD9,
            ];
            temp_file.write_all(&jpeg_data).unwrap();
            (temp_file, jpeg_data)
        }
    }

    #[actix_web::test]
    async fn test_post_photos_upload_new_file() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let (_temp_file, jpeg_data) = TestContext::create_test_jpeg();
        let boundary = "----formdata-test-boundary";

        let multipart_body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"test.jpg\"\r\n\
             Content-Type: image/jpeg\r\n\r\n\
             {jpeg_content}\r\n\
             --{boundary}--\r\n",
            boundary = boundary,
            jpeg_content = String::from_utf8_lossy(&jpeg_data)
        );

        let req = test::TestRequest::post()
            .uri("/api/photos")
            .insert_header((
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            ))
            .set_payload(Bytes::from(multipart_body))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 201);

        let body: Value = test::read_body_json(resp).await;
        assert!(body["id"].as_i64().unwrap() > 0);
        assert_eq!(body["filename"], "test.jpg");
        assert_eq!(body["mime_type"], "image/jpeg");
    }

    #[actix_web::test]
    async fn test_post_photos_upload_invalid_file() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let boundary = "----formdata-test-boundary";
        let invalid_data = "not-an-image";

        let multipart_body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"invalid.txt\"\r\n\
             Content-Type: text/plain\r\n\r\n\
             {content}\r\n\
             --{boundary}--\r\n",
            boundary = boundary,
            content = invalid_data
        );

        let req = test::TestRequest::post()
            .uri("/api/photos")
            .insert_header((
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            ))
            .set_payload(Bytes::from(multipart_body))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 400);

        let body: Value = test::read_body_json(resp).await;
        assert!(body["error"]
            .as_str()
            .unwrap()
            .contains("Invalid image format"));
    }

    #[actix_web::test]
    async fn test_put_photos_update_metadata() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let photo_id = ctx.create_test_photo("/test/update.jpg", "update.jpg");

        let update_data = json!({
            "filename": "updated_name.jpg",
            "camera_make": "Updated Camera",
            "camera_model": "Updated Model",
            "location_name": "Updated Location"
        });

        let req = test::TestRequest::put()
            .uri(&format!("/api/photos/{}", photo_id))
            .insert_header(("content-type", "application/json"))
            .set_json(&update_data)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["id"], photo_id);
        assert_eq!(body["filename"], "updated_name.jpg");
        assert_eq!(body["camera_make"], "Updated Camera");
        assert_eq!(body["camera_model"], "Updated Model");
        assert_eq!(body["location_name"], "Updated Location");
    }

    #[actix_web::test]
    async fn test_put_photos_not_found() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let update_data = json!({
            "filename": "not_found.jpg"
        });

        let req = test::TestRequest::put()
            .uri("/api/photos/99999")
            .insert_header(("content-type", "application/json"))
            .set_json(&update_data)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);

        let body: Value = test::read_body_json(resp).await;
        assert!(body["error"].as_str().unwrap().contains("Photo not found"));
    }

    #[actix_web::test]
    async fn test_delete_photos_success() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let photo_id = ctx.create_test_photo("/test/delete.jpg", "delete.jpg");

        // Verify photo exists first
        let req = test::TestRequest::get()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        // Delete the photo
        let req = test::TestRequest::delete()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 204);

        // Verify photo is deleted
        let req = test::TestRequest::get()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }

    #[actix_web::test]
    async fn test_delete_photos_not_found() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::delete()
            .uri("/api/photos/99999")
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);

        let body: Value = test::read_body_json(resp).await;
        assert!(body["error"].as_str().unwrap().contains("Photo not found"));
    }

    #[actix_web::test]
    async fn test_crud_workflow_complete() {
        let ctx = TestContext::new();
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(ctx.pool.clone()))
                .configure(configure_routes),
        )
        .await;

        // 1. Create a test photo through database (simulating successful upload)
        let photo_id = ctx.create_test_photo("/test/workflow.jpg", "workflow.jpg");

        // 2. Get the photo
        let req = test::TestRequest::get()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        // 3. Update the photo
        let update_data = json!({
            "camera_make": "Workflow Camera",
            "location_name": "Test Location"
        });

        let req = test::TestRequest::put()
            .uri(&format!("/api/photos/{}", photo_id))
            .insert_header(("content-type", "application/json"))
            .set_json(&update_data)
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: Value = test::read_body_json(resp).await;
        assert_eq!(body["camera_make"], "Workflow Camera");
        assert_eq!(body["location_name"], "Test Location");

        // 4. List photos should include our photo
        let req = test::TestRequest::get().uri("/api/photos").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 200);

        let body: Value = test::read_body_json(resp).await;
        assert!(body["photos"].as_array().unwrap().len() > 0);

        // 5. Delete the photo
        let req = test::TestRequest::delete()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();

        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 204);

        // 6. Verify deletion
        let req = test::TestRequest::get()
            .uri(&format!("/api/photos/{}", photo_id))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 404);
    }
}

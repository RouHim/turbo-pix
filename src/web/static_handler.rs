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
            _ => Self::NotFound,
        }
    }

    pub fn content(&self) -> Option<&'static str> {
        match self {
            Self::IndexHtml => Some(include_str!("../../static/index.html")),
            Self::MainCss => Some(include_str!("../../static/css/main.css")),
            Self::ComponentsCss => Some(include_str!("../../static/css/components.css")),
            Self::ResponsiveCss => Some(include_str!("../../static/css/responsive.css")),
            Self::AppJs => Some(include_str!("../../static/js/app.js")),
            Self::ApiJs => Some(include_str!("../../static/js/api.js")),
            Self::LoggerJs => Some(include_str!("../../static/js/logger.js")),
            Self::PhotoGridJs => Some(include_str!("../../static/js/photoGrid.js")),
            Self::ViewerJs => Some(include_str!("../../static/js/viewer.js")),
            Self::SearchJs => Some(include_str!("../../static/js/search.js")),
            Self::UtilsJs => Some(include_str!("../../static/js/utils.js")),
            Self::NotFound => None,
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::IndexHtml => "text/html; charset=utf-8",
            Self::MainCss | Self::ComponentsCss | Self::ResponsiveCss => "text/css; charset=utf-8",
            Self::AppJs
            | Self::ApiJs
            | Self::LoggerJs
            | Self::PhotoGridJs
            | Self::ViewerJs
            | Self::SearchJs
            | Self::UtilsJs => "application/javascript; charset=utf-8",
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
    let asset = StaticAsset::from_path(path);

    match asset.content() {
        Some(content) => Ok(HttpResponse::Ok()
            .content_type(asset.mime_type())
            .body(content)),
        None => Ok(HttpResponse::NotFound()
            .content_type("text/plain")
            .body("File not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let app =
            test::init_service(App::new().route("/{path:.*}", web::get().to(serve_static_asset)))
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
        assert!(content.unwrap().contains("<!doctype html>"));

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

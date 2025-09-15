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

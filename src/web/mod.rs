pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod static_handler;

#[cfg(test)]
mod api_tests {
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

        let (temp_file, jpeg_data) = TestContext::create_test_jpeg();
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

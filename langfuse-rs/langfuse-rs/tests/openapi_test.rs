use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(info(title = "Test", version = "0.1"))]
struct TestDoc;

#[test]
fn test_openapi() {
    let spec = TestDoc::openapi();
    assert_eq!(spec.info.title, "Test");
}

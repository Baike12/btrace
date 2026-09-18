
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDoc;

#[derive(OpenApi)]
#[openapi(info(title = "T", version = "0.1"))]
struct T;

fn main() {
    let spec: OpenApiDoc = T::openapi();
    println!("{:?}", spec.info.title);
}

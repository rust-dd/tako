#![cfg(any(feature = "utoipa", feature = "vespera"))]

use http_body_util::BodyExt;
use tako_rs_core::openapi::ui::SwaggerUi;
use tako_rs_core::responder::Responder;

#[tokio::test]
async fn swagger_url_cannot_close_inline_script_or_inject_html() {
  for url in [
    "/openapi.json</script><script>alert(1)</script>",
    "/openapi.json</ScRiPt ><img src=x onerror=alert(1)>",
    "/openapi.json<!--<script>unclosed",
    "/openapi.json?value=\"quoted\"&line=\n\\unicode\u{2028}\u{2029}",
  ] {
    let response = SwaggerUi::new(url).into_response();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(bytes.to_vec()).unwrap();

    assert_eq!(
      html.to_ascii_lowercase().matches("</script").count(),
      2,
      "{url}"
    );
    assert_eq!(
      html.to_ascii_lowercase().matches("<script").count(),
      2,
      "{url}"
    );
    let literal = html
      .lines()
      .find_map(|line| line.trim().strip_prefix("url: "))
      .unwrap();
    let literal = literal.strip_suffix(',').unwrap();
    assert!(!literal.contains('<'), "{literal}");
    assert_eq!(serde_json::from_str::<String>(literal).unwrap(), url);
  }
}

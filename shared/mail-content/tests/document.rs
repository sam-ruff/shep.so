use shep_mail_content::document::{self, Options};
fn options() -> Options {
    Options {
        generation: "fixture-generation".into(),
        dark: false,
        quotes: false,
    }
}

#[test]
fn formatted_document_preserves_styles_tables_and_one_bounded_inline_blob() {
    let prepared =
        document::prepare(include_bytes!("../../html-reader-fixture.eml"), &options()).unwrap();
    assert!(prepared.text.starts_with("Plain alternative"));
    assert_eq!(prepared.remote_images.len(), 1);
    assert_eq!(
        prepared.remote_images[0].url,
        "https://images.example.test/news/banner.webp"
    );
    assert!(prepared.issues.is_empty(), "{:?}", prepared.issues);
    let document = prepared.document.unwrap();
    let html = scraper::Html::parse_document(&document);
    let data = html
        .select(&scraper::Selector::parse("#shep-data").unwrap())
        .next()
        .unwrap()
        .text()
        .collect::<String>();
    let data: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(data["images"].as_object().unwrap().len(), 1);
    assert_eq!(data["links"]["0"], "https://example.test/help");
    assert_eq!(
        html.select(&scraper::Selector::parse("table.card").unwrap())
            .count(),
        1
    );
    assert!(document.contains("background:#101014"));
    assert!(document.contains("urn:shep-image:"));
    assert!(!document.contains("href=\"https://example.test/help\""));
    assert!(document.contains("Content-Security-Policy"));
}

#[test]
fn sender_active_content_is_removed_and_css_resources_are_inventory_only() {
    let source = r#"Content-Type: text/html

<base href="https://images.example.test/path/"><style>
@import 'https://bad.example.test/import.css'; @font-face{src:url(https://bad.example.test/font)}
@media screen{.escaped{background:u\72l('../escaped.webp')}}
.set{background:image-set("https://images.example.test/one.webp" 1x,url(https://images.example.test/two.webp) 2x)}
</style><script>SECRET_ATTACK()</script><iframe src="https://bad.example.test/frame"></iframe>
<meta http-equiv=refresh content="0;url=https://bad.example.test/nav"><link rel=stylesheet href="https://bad.example.test/css">
<form action="https://bad.example.test/post"><input autofocus name=password></form>
<img src="https://images.example.test/img.webp" onerror="SECRET_ATTACK()" srcset="https://images.example.test/large.webp 2x">
<a href="javascript:SECRET_ATTACK()">Unsafe</a><a href="https://example.test/safe" ping="https://bad.example.test/ping" target="_top">Safe</a>
<svg onload="SECRET_ATTACK()"><foreignObject><script>SECRET_ATTACK()</script></foreignObject></svg>"#;
    let result = document::prepare(source.as_bytes(), &options()).unwrap();
    let html = result.document.unwrap();
    assert!(!html.contains("SECRET_ATTACK"));
    assert!(!html.contains("bad.example.test"));
    let urls: Vec<_> = result
        .remote_images
        .iter()
        .map(|image| image.url.as_str())
        .collect();
    assert_eq!(
        urls,
        vec![
            "https://images.example.test/escaped.webp",
            "https://images.example.test/img.webp",
            "https://images.example.test/large.webp",
            "https://images.example.test/one.webp",
            "https://images.example.test/two.webp"
        ]
    );
    assert_eq!(
        html.matches("<script").count(),
        2,
        "Only the owned JSON and display runtime remain"
    );
    assert!(!html.contains("<iframe"));
    assert!(!html.contains("<form"));
}

#[test]
fn css_string_decoding_cannot_create_html_end_tags() {
    let source = r#"Content-Type: text/html

<style>.attack{content:"\3c /style>\3c script>ATTACK\3c /script>"}</style><p>Still text</p>"#;
    let html = document::prepare(source.as_bytes(), &options())
        .unwrap()
        .document
        .unwrap();
    let parsed = scraper::Html::parse_document(&html);
    assert_eq!(
        parsed
            .select(&scraper::Selector::parse("script").unwrap())
            .count(),
        2
    );
    assert!(html.contains("<\\/style>"));
}

#[test]
fn cid_data_and_css_images_share_one_converted_resource_and_invalid_images_are_visible() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let bytes = include_bytes!("../../../assets/logo-light.webp");
    let encoded = STANDARD.encode(bytes);
    let raw = format!(
        "Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<style>.image{{background:url(CID:dog)}}</style><div class=image><img src='cId:dog'><img src='DATA:IMAGE/WEBP;base64,{encoded}' srcset='cid:dog 1x, cid:dog 2x'><img src='data:image/png;base64,bm90LWFuLWltYWdl'></div>\r\n--x\r\nContent-Type: image/webp\r\nContent-ID: <dog>\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n--x--\r\n"
    );
    let prepared = document::prepare(raw.as_bytes(), &options()).unwrap();
    assert_eq!(prepared.issues.len(), 1);
    assert!(prepared.remote_images.is_empty());
    let tree = scraper::Html::parse_document(prepared.document.as_ref().unwrap());
    let json = tree
        .select(&scraper::Selector::parse("#shep-data").unwrap())
        .next()
        .unwrap()
        .text()
        .collect::<String>();
    let data: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(data["images"].as_object().unwrap().len(), 1);
    let id = data["images"].as_object().unwrap().keys().next().unwrap();
    assert_eq!(
        prepared
            .document
            .unwrap()
            .matches(&format!("urn:shep-image:{id}"))
            .count(),
        5
    );
}

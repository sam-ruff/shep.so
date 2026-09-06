//! Fictional styled mail exercises the same MIME/store/reader path as real mail.
use crate::{model::parse_mail, store::Store};
use base64::Engine;

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let logo = base64::engine::general_purpose::STANDARD
        .encode(include_bytes!("../../assets/logo-light.webp"));
    let styled = format!(r##"Content-Type: multipart/related; boundary=htmlfixture

--htmlfixture
Content-Type: multipart/alternative; boundary=choices

--choices
Content-Type: text/plain; charset=utf-8

Your sample sign-in request. This is the plain text alternative.
--choices
Content-Type: text/html; charset=utf-8

<!doctype html><html><head><style>
body {{ margin:0; background:#101010; color:#ededed; font-family:Arial,sans-serif; }}
.card {{border:1px solid #303030; width:100%; border-collapse:collapse;}}
td {{padding:24px;}} h1 {{font-size:28px; margin:12px 0 22px;}} p {{line-height:1.6;}}
.code {{background:#202020;color:#a4d8ce;padding:18px;text-align:center;font-size:24px;font-weight:bold;}}
a {{color:#92c9ff;}}
</style></head><body><table class="card"><tr><td>
<img src="cid:shepherd%40fixture" width="48" height="48" style="vertical-align:middle"> <b>Example Domain Service</b>
<h1>Verification needed</h1><p>Please confirm your sample sign-in request.</p>
<p>We noticed a sign-in from a new device.</p>
<ul><li>Account: sample-reader</li><li>Device: Fictional desktop</li><li>Location: Example City</li></ul>
<div class="code">SAMPLE-ONLY</div>
<p>If this was you, continue from your browser.</p>
<p><a href="https://example.test/help">Visit the help centre</a></p>
<img src="https://images.example.test/banner.webp" width="96" height="48" alt="External banner">
<blockquote><p>Earlier message: the original request.</p></blockquote>
</td></tr></table></body></html>
--choices--
--htmlfixture
Content-Type: image/webp
Content-ID: <shepherd@fixture>
Content-Transfer-Encoding: base64

{logo}
--htmlfixture--
"##).replace('\n', "\r\n");
    let xhtml = "Content-Type: text/plain; charset=utf-8\r\n\r\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Sample request</title></head><body><p>Hello sample reader,</p><p>Your fictional authorization code is: <b>SAMPLE-ONLY</b>.</p><p>Domain: example.test</p><p>Regards,<br />Example support</p></body></html>".to_owned();
    let escaped = format!(
        "Content-Type: text/plain; charset=utf-8\r\n\r\n{}",
        crate::email_content::escape(
            "<html><body><h2>Escaped HTML request</h2><p>Readable content without raw tags.</p></body></html>"
        )
    );
    let long = format!(
        "Content-Type: text/html; charset=utf-8\r\n\r\n<html><body><h1>Long formatted letter</h1>{}<p>Last visible paragraph.</p></body></html>",
        (0..200)
            .map(|i| format!(
                "<p>Paragraph {i}: a long message remains scrollable and selectable.</p>"
            ))
            .collect::<String>()
    );
    for (index, subject, body) in [
        (0, "Styled sign-in sample", styled),
        (1, "Mislabeled XHTML request", xhtml),
        (2, "Escaped HTML request", escaped),
        (3, "Long formatted letter", long),
    ] {
        let raw = format!(
            "From: Example Support <support@example.test>\r\nTo: Alex <alex@studio.example>\r\nSubject: {subject}\r\nMessage-ID: <html-{index}@example.test>\r\n{body}"
        );
        let mut mail = parse_mail(
            "preview-work",
            &format!("html-{index}"),
            "INBOX",
            raw.into_bytes(),
            true,
            false,
        )?;
        mail.summary.timestamp = chrono::Utc::now().timestamp() + 100 - index;
        store.upsert(vec![mail]).await?;
    }
    let raw = b"From: Reports <reports@example.test>\r\nTo: alex@studio.example\r\nSubject: Wide HTML report\r\nContent-Type: text/html\r\n\r\n<html><body><table style=\"width:1000px;border-collapse:collapse\"><tr><td style=\"width:500px;background:#c5d8f5;padding:20px\">Left report column</td><td style=\"width:500px;background:#f3dab0;padding:20px\">Right report column</td></tr></table></body></html>";
    store
        .upsert(vec![parse_mail(
            "preview-work",
            "html-wide",
            "Projects",
            raw.to_vec(),
            false,
            false,
        )?])
        .await?;
    let css = b"From: Reports <reports@example.test>\r\nTo: alex@studio.example\r\nSubject: CSS background report\r\nContent-Type: text/html\r\n\r\n<html><head><base href=\"https://images.example.test/reports/\"><style>.banner{height:96px;background:#e2e8f0 url('../report.webp') 12px center/48px 48px no-repeat;padding-left:80px;line-height:96px}</style></head><body><div class=\"banner\">Project overview</div><table style=\"width:1000px;border-collapse:collapse\"><tr><td style=\"width:500px;padding:20px\">First report column</td><td style=\"width:500px;padding:20px\">Last report column</td></tr></table></body></html>";
    store
        .upsert(vec![parse_mail(
            "preview-work",
            "html-css",
            "Archive",
            css.to_vec(),
            false,
            false,
        )?])
        .await?;
    Ok(())
}

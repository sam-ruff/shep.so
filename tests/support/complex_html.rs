//! Fully fictional utility CSS and deeply nested presentation tables. The depth
//! mirrors a common mail-template structure; no personal HTML is embedded.
pub fn letter(depth: usize) -> String {
    let css = (0..1182)
        .map(|i| format!(".utility-{i}{{padding:4px;margin:0;color:#18181b}}\n"))
        .collect::<String>();
    let mut body = "<div style='position:relative'><div style='position:absolute;right:4px;top:2px'>Delivery</div><h1>Your workshop delivery <sup style='position:relative;top:2px'>42</sup></h1><p><span style='position:relative;left:0;right:8px;top:0;bottom:6px'>Fictional tools are on their way.</span></p><table width='100%'><tr><td>Item</td><td>Quantity</td></tr><tr><td>Workshop cable</td><td>1</td></tr></table></div>".to_owned();
    for index in 0..depth {
        body = format!(
            "<table width='100%' cellspacing='0' cellpadding='0'><tr><td><div><div class='utility-{}'>{body}</div></div></td></tr></table>",
            index * 7
        );
    }
    format!(
        "<html><head><style>{css}</style></head><body><table width='600' align='center'><tr><td>{body}</td></tr></table></body></html>"
    )
}

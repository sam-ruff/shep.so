//! Iterative text fallback: HTML nesting must not recurse on a native/WASM stack.
pub(crate) fn html_to_text(html: &str) -> String {
    let document = scraper::Html::parse_document(html);
    let mut output = String::new();
    let mut stack = vec![(document.root_element().id(), false, false)];
    while let Some((id, closing, quoted)) = stack.pop() {
        let node = document.tree.get(id).expect("node from the same document");
        if let Some(element) = scraper::ElementRef::wrap(node) {
            let name = element.value().name();
            if matches!(name, "style" | "script" | "head" | "title" | "noscript") {
                continue;
            }
            let block = matches!(
                name,
                "p" | "div" | "br" | "tr" | "li" | "blockquote" | "h1" | "h2" | "h3"
            );
            if block && !output.ends_with('\n') {
                output.push('\n');
            }
            if !closing {
                stack.push((id, true, quoted));
                for child in element.children().rev() {
                    stack.push((child.id(), false, quoted || name == "blockquote"));
                }
            }
        } else if let Some(text) = node.value().as_text() {
            let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() {
                if output.ends_with('\n') && quoted {
                    output.push_str("> ");
                } else if !output.ends_with(['\n', ' ']) && !output.is_empty() {
                    output.push(' ');
                }
                output.push_str(&normalized);
            }
        }
    }
    output.trim().into()
}

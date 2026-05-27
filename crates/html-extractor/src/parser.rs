//! HTML parser: drives html5ever + markup5ever_rcdom into our internal
//! [`crate::tree::Tree`].

use compact_str::CompactString;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::tree::{Element, Tree};
use crate::types::ExtractError;

/// Parse an HTML document into our internal tree.
pub(crate) fn parse(html: &str) -> Result<Tree, ExtractError> {
    let dom = html5ever::parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .map_err(|_| ExtractError::ParseFailure("html5ever I/O error"))?;

    let mut tree = Tree::default();
    // Reserve a synthetic document root.
    tree.nodes.push(Element {
        tag: "#document".into(),
        parent: usize::MAX,
        ..Element::default()
    });
    tree.root = 0;
    tree.head = usize::MAX;
    tree.body = 0;

    walk(&dom.document, 0, &mut tree);

    // Find body if present; else fall back to root.
    if let Some(body) = find_tag(&tree, tree.root, "body") {
        tree.body = body;
    }
    if let Some(head) = find_tag(&tree, tree.root, "head") {
        tree.head = head;
        // Title text is collected eagerly so it's preserved across the
        // pre-clean stage that drops <head>.
        tree.title_text = first_title_text(&tree, head);
    }
    if let Some(html_idx) = find_tag(&tree, tree.root, "html") {
        if let Some(lang) = tree.get(html_idx).attr("lang") {
            tree.html_lang = Some(lang.to_string());
        }
    }

    Ok(tree)
}

fn walk(node: &Handle, parent: usize, tree: &mut Tree) {
    let children = node.children.borrow();
    for child in children.iter() {
        match &child.data {
            NodeData::Element { name, attrs, .. } => {
                let tag = CompactString::from(name.local.as_ref().to_lowercase());
                let mut attrs_vec = Vec::with_capacity(attrs.borrow().len());
                for a in attrs.borrow().iter() {
                    attrs_vec.push((a.name.local.to_string().to_lowercase(), a.value.to_string()));
                }
                let class_id = Element::compute_class_id(&attrs_vec);
                let idx = tree.nodes.len();
                tree.nodes.push(Element {
                    tag,
                    attrs: attrs_vec,
                    own_text: String::new(),
                    class_id,
                    children: Vec::new(),
                    parent,
                });
                tree.nodes[parent].children.push(idx);
                walk(child, idx, tree);
            }
            NodeData::Text { contents } => {
                let s = contents.borrow();
                let text = s.as_ref();
                if !text.trim().is_empty() && parent != usize::MAX {
                    if !tree.nodes[parent].own_text.is_empty()
                        && !tree.nodes[parent].own_text.ends_with(' ')
                    {
                        tree.nodes[parent].own_text.push(' ');
                    }
                    tree.nodes[parent].own_text.push_str(text);
                }
            }
            NodeData::Comment { .. } => {
                // dropped at parse time, satisfies the Stage-1 "drop HTML
                // comments" requirement cheaply
            }
            NodeData::Document
            | NodeData::Doctype { .. }
            | NodeData::ProcessingInstruction { .. } => {
                walk(child, parent, tree);
            }
        }
    }
}

fn find_tag(tree: &Tree, root: usize, tag: &str) -> Option<usize> {
    let mut found = None;
    tree.walk_pre(root, |idx| {
        if found.is_some() {
            return false;
        }
        if tree.get(idx).tag == tag {
            found = Some(idx);
            return false;
        }
        true
    });
    found
}

fn first_title_text(tree: &Tree, head: usize) -> String {
    if let Some(t) = find_tag(tree, head, "title") {
        tree.full_text(t).trim().to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_document() {
        let tree = parse("<html><body><p>hi</p></body></html>").unwrap();
        assert!(tree.nodes.iter().any(|n| n.tag == "p"));
        assert!(tree.body != usize::MAX);
    }

    #[test]
    fn drops_html_comments() {
        let tree = parse("<html><body><!-- x --><p>hi</p></body></html>").unwrap();
        // Comments shouldn't show up as own_text on any element.
        for n in &tree.nodes {
            assert!(!n.own_text.contains("x"));
        }
    }

    #[test]
    fn collects_title_text_eagerly() {
        let tree = parse("<html><head><title>My Title</title></head><body><p>x</p></body></html>")
            .unwrap();
        assert_eq!(tree.title_text, "My Title");
    }

    #[test]
    fn collects_html_lang() {
        let tree = parse("<html lang='fr'><body><p>x</p></body></html>").unwrap();
        assert_eq!(tree.html_lang.as_deref(), Some("fr"));
    }

    #[test]
    fn malformed_html_does_not_panic() {
        let _ = parse("<html><body><div><p>hi");
    }
}

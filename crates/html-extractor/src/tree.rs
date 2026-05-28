//! Internal arena tree built from rcdom.
//!
//! See `DECISIONS.md` D3: we avoid mutating rcdom in place. Instead we walk
//! the rcdom once and copy the relevant nodes into a flat `Vec<Node>` that's
//! cheap to iterate.

/// Element-only payload (text and other text-like nodes are inlined as
/// children of their owning element to keep the iteration uniform).
#[derive(Debug, Clone, Default)]
pub(crate) struct Element {
    /// Lower-case tag name (`"div"`, `"p"`, …). The root node uses
    /// `"#document"`.
    pub tag: String,
    /// All attributes on the element. Names are lowercased.
    pub attrs: Vec<(String, String)>,
    /// Direct text content owned by this element (concatenated from any
    /// adjacent text-node children). Useful for leaf-level scoring.
    pub own_text: String,
    /// The `class` and `id` attribute values joined and lowercased, cached at
    /// parse time (empty when the element has neither). Recomputing this per
    /// walk was a measurable allocation hotspot — the classifier, scoring, and
    /// clean passes each call it for every node — so it's materialized once.
    pub class_id: String,
    /// Indices of child elements in `Tree.nodes`.
    pub children: Vec<usize>,
    /// Index of parent (`usize::MAX` for the root).
    pub parent: usize,
}

impl Element {
    /// The cached lowercased `class`+`id` string (see [`Element::class_id`]),
    /// matched against chrome / content / page-type hint regexes.
    pub fn class_id_lower(&self) -> &str {
        &self.class_id
    }

    /// Compute the lowercased `class`+`id` join from an attribute list. The
    /// parser uses this to populate [`Element::class_id`] exactly once.
    pub fn compute_class_id(attrs: &[(String, String)]) -> String {
        let mut out = String::new();
        for (k, v) in attrs {
            if k == "class" || k == "id" {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(v);
            }
        }
        out.to_lowercase()
    }

    /// CSS-selector-shaped signature for this element: `tag` + sorted
    /// `.class`es + `#id` (e.g. `div.related.sidebar#aside`). Used by the
    /// decisions ledger; deterministic so it aggregates across pages.
    pub fn selector(&self) -> String {
        let mut s = self.tag.to_string();
        if let Some(class) = self.attr("class") {
            let mut classes: Vec<&str> = class.split_whitespace().collect();
            classes.sort_unstable();
            for c in classes {
                s.push('.');
                s.push_str(c);
            }
        }
        if let Some(id) = self.attr("id") {
            if !id.trim().is_empty() {
                s.push('#');
                s.push_str(id.trim());
            }
        }
        s
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn has_attr(&self, name: &str) -> bool {
        self.attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    }
}

/// Per-node text metrics for a subtree, indexed by node id. See
/// [`Tree::subtree_text_metrics`].
pub(crate) struct SubtreeTextMetrics {
    /// `full_text(idx).chars().count()` for each node.
    pub chars: Vec<usize>,
    /// Total `<a>` own-text chars in each node's subtree.
    pub link_chars: Vec<usize>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Tree {
    pub nodes: Vec<Element>,
    /// Index of `<html>` (or first element after the document root).
    pub root: usize,
    /// Index of `<body>` if present, else `root`.
    pub body: usize,
    /// Index of `<head>` if present, else `usize::MAX`.
    pub head: usize,
    /// Original concatenated text from `<title>` (used by metadata).
    pub title_text: String,
    /// Original `<html lang="">` value, if any.
    pub html_lang: Option<String>,
}

impl Tree {
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn get(&self, idx: usize) -> &Element {
        &self.nodes[idx]
    }

    pub fn get_mut(&mut self, idx: usize) -> &mut Element {
        &mut self.nodes[idx]
    }

    /// Walk every descendant of `root` post-order, calling `f`.
    pub fn walk_post<F: FnMut(usize)>(&self, root: usize, mut f: F) {
        // Iterative post-order to avoid deep recursion on pathological inputs.
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((idx, child_i)) = stack.last_mut().copied() {
            let node = &self.nodes[idx];
            if child_i < node.children.len() {
                let child = node.children[child_i];
                stack.last_mut().unwrap().1 += 1;
                stack.push((child, 0));
            } else {
                f(idx);
                stack.pop();
            }
        }
    }

    /// Walk pre-order (parent before children). Useful for fast-path
    /// searching by tag or class.
    pub fn walk_pre<F: FnMut(usize) -> bool>(&self, root: usize, mut f: F) {
        let mut stack: Vec<usize> = vec![root];
        while let Some(idx) = stack.pop() {
            if !f(idx) {
                continue;
            }
            for &child in self.nodes[idx].children.iter().rev() {
                stack.push(child);
            }
        }
    }

    /// Mark a node as semantically dropped (we don't reallocate; we set its
    /// tag to a sentinel `_dropped_` so descendant walks can skip it).
    pub fn drop_subtree(&mut self, idx: usize) {
        let mut stack = vec![idx];
        while let Some(i) = stack.pop() {
            self.nodes[i].tag = "_dropped_".to_string();
            self.nodes[i].own_text.clear();
            for &c in &self.nodes[i].children.clone() {
                stack.push(c);
            }
        }
    }

    /// Number of UTF-8 characters in the subtree's text, NOT counting text
    /// inside `<a>` descendants (matches `text_length` feature description in
    /// ALGORITHM.md).
    pub fn text_len_excluding_links(&self, idx: usize) -> usize {
        let mut total = 0usize;
        self.walk_subtree_text(idx, &mut |elem| {
            if elem.tag == "a" {
                return false; // skip the entire link subtree
            }
            total += elem.own_text.chars().count();
            true
        });
        total
    }

    /// Internal helper that walks descendants and feeds each element to `f`.
    /// Return `false` from `f` to skip a subtree.
    pub fn walk_subtree_text<F: FnMut(&Element) -> bool>(&self, idx: usize, f: &mut F) {
        let mut stack = vec![idx];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i];
            if node.tag == "_dropped_" {
                continue;
            }
            let descend = f(node);
            if descend {
                for &c in node.children.iter().rev() {
                    stack.push(c);
                }
            }
        }
    }

    /// One post-order pass computing, for every node in `root`'s subtree, the
    /// exact `full_text(idx).chars().count()` and the total `<a>` own-text
    /// chars in the subtree. Replaces per-node `full_text` calls in post-clean,
    /// which were O(N²) (each node re-walked its whole subtree). Indices
    /// outside `root`'s subtree are left at 0.
    ///
    /// The char count is composed with the same separator rule as
    /// [`full_text`](Self::full_text): a single space is inserted before a
    /// non-empty text run when the running buffer is non-empty and does not
    /// already end in whitespace. Dropped subtrees contribute nothing.
    pub fn subtree_text_metrics(&self, root: usize) -> SubtreeTextMetrics {
        #[derive(Clone, Copy, Default)]
        struct Agg {
            chars: usize,
            link_chars: usize,
            any: bool,
            last_ends_ws: bool,
        }
        let mut agg = vec![Agg::default(); self.nodes.len()];
        self.walk_post(root, |idx| {
            let node = &self.nodes[idx];
            if node.tag == "_dropped_" {
                agg[idx] = Agg::default();
                return;
            }
            let mut a = Agg::default();
            if node.tag == "a" {
                a.link_chars = node.own_text.chars().count();
            }
            // own_text comes first in full_text's pre-order traversal.
            if !node.own_text.is_empty() {
                a.chars = node.own_text.chars().count();
                a.any = true;
                a.last_ends_ws = node.own_text.ends_with(char::is_whitespace);
            }
            for &c in &node.children {
                let ca = agg[c];
                a.link_chars += ca.link_chars;
                if ca.any {
                    if a.any && !a.last_ends_ws {
                        a.chars += 1; // separator space
                    }
                    a.chars += ca.chars;
                    a.any = true;
                    a.last_ends_ws = ca.last_ends_ws;
                }
            }
            agg[idx] = a;
        });
        SubtreeTextMetrics {
            chars: agg.iter().map(|a| a.chars).collect(),
            link_chars: agg.iter().map(|a| a.link_chars).collect(),
        }
    }

    /// Concatenated descendant text (no link exclusion). Used for link-density
    /// math and minimum-content checks.
    pub fn full_text(&self, idx: usize) -> String {
        let mut buf = String::new();
        self.walk_subtree_text(idx, &mut |elem| {
            if !elem.own_text.is_empty() {
                if !buf.is_empty() && !buf.ends_with(char::is_whitespace) {
                    buf.push(' ');
                }
                buf.push_str(&elem.own_text);
            }
            true
        });
        buf
    }

    /// Descendant tag count (excluding `idx` itself, excluding dropped).
    #[allow(dead_code)]
    pub fn descendant_tag_count(&self, idx: usize) -> usize {
        let mut n = 0usize;
        self.walk_subtree_text(idx, &mut |elem| {
            if elem.tag == "_dropped_" {
                false
            } else {
                n += 1;
                true
            }
        });
        n.saturating_sub(1) // don't count self
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    fn link_chars_via_walk(tree: &crate::tree::Tree, idx: usize) -> usize {
        let mut n = 0usize;
        tree.walk_subtree_text(idx, &mut |elem| {
            if elem.tag == "a" {
                n += elem.own_text.chars().count();
            }
            elem.tag != "_dropped_"
        });
        n
    }

    #[test]
    fn subtree_text_metrics_match_full_text_for_every_node() {
        // Mix of nested wrappers, sibling text runs, links, and whitespace
        // boundaries — the cases where the separator rule matters.
        let html = "<html><body>\
            <div id=\"a\">Hello <span>world</span> and <a href=\"/x\">a link</a> here.\
                <p>Second paragraph with <a href=\"/y\">another link</a> inside it.</p>\
            </div>\
            <ul><li>one</li><li>two</li><li><a href=\"/z\">three</a></li></ul>\
            </body></html>";
        let tree = parse(html).unwrap();
        let metrics = tree.subtree_text_metrics(tree.root);
        for idx in 0..tree.len() {
            assert_eq!(
                metrics.chars[idx],
                tree.full_text(idx).chars().count(),
                "chars mismatch at node {idx} (tag {:?})",
                tree.get(idx).tag
            );
            assert_eq!(
                metrics.link_chars[idx],
                link_chars_via_walk(&tree, idx),
                "link_chars mismatch at node {idx} (tag {:?})",
                tree.get(idx).tag
            );
        }
    }
}

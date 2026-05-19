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
    /// Indices of child elements in `Tree.nodes`.
    pub children: Vec<usize>,
    /// Index of parent (`usize::MAX` for the root).
    pub parent: usize,
}

impl Element {
    /// Return the `class` attribute joined with the `id` attribute, lowercased
    /// for regex matching.
    pub fn class_id_lower(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.attrs {
            if k == "class" || k == "id" {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(v);
            }
        }
        out.to_lowercase()
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

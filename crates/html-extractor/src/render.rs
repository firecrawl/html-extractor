//! Stage 5 — markdown renderer for the chosen subtree.

use crate::clean::{is_chrome, is_share_or_ad, CleanedRoot};
use crate::tree::{Element, Tree};
use crate::types::ExtractOptions;

/// Render the cleaned subtree as GitHub-flavored markdown. Returns
/// `(markdown, text_char_count)`.
pub(crate) fn render(
    tree: &Tree,
    cleaned: &CleanedRoot,
    options: &ExtractOptions,
) -> (String, usize) {
    let mut ctx = RenderCtx {
        out: String::with_capacity(2048),
        list_stack: Vec::new(),
        in_pre: false,
        in_link: 0,
        skip: &cleaned.skip,
        options,
        text_chars: 0,
        last_was_block: true,
    };
    render_node(tree, cleaned.root, &mut ctx);
    let md = normalize_whitespace(&ctx.out);
    let count = md.chars().count();
    (md, count.max(ctx.text_chars))
}

struct RenderCtx<'a> {
    out: String,
    list_stack: Vec<ListFrame>,
    in_pre: bool,
    in_link: usize,
    skip: &'a std::collections::HashSet<usize>,
    options: &'a ExtractOptions,
    text_chars: usize,
    last_was_block: bool,
}

struct ListFrame {
    ordered: bool,
    counter: usize,
}

fn render_node(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    let elem = tree.get(idx);
    if elem.tag == "_dropped_" {
        return;
    }
    if ctx.skip.contains(&idx) {
        return;
    }
    // Drop residual chrome that snuck into the kept subtree.
    let needle = elem.class_id_lower();
    if !needle.is_empty() && (is_chrome(&needle) || is_share_or_ad(&needle)) {
        return;
    }

    match elem.tag.as_str() {
        "h1" => render_heading(tree, idx, 1, ctx),
        "h2" => render_heading(tree, idx, 2, ctx),
        "h3" => render_heading(tree, idx, 3, ctx),
        "h4" => render_heading(tree, idx, 4, ctx),
        "h5" => render_heading(tree, idx, 5, ctx),
        "h6" => render_heading(tree, idx, 6, ctx),
        "p" => render_block_paragraph(tree, idx, ctx),
        "br" => {
            if !ctx.in_pre {
                ctx.out.push_str("  \n");
            } else {
                ctx.out.push('\n');
            }
        }
        "hr" => {
            block_break(ctx);
            ctx.out.push_str("---\n\n");
            ctx.last_was_block = true;
        }
        "blockquote" => render_blockquote(tree, idx, ctx),
        "pre" => render_pre(tree, idx, ctx),
        "code" => render_code_inline(tree, idx, ctx),
        "ul" => render_list(tree, idx, false, ctx),
        "ol" => render_list(tree, idx, true, ctx),
        "li" => render_list_item(tree, idx, ctx),
        "table" if ctx.options.include_tables => render_table(tree, idx, ctx),
        "img" if ctx.options.include_images => render_image(elem, ctx),
        "a" if ctx.options.include_links => render_link(tree, idx, elem, ctx),
        "strong" | "b" => render_wrap(tree, idx, "**", "**", ctx),
        "em" | "i" => render_wrap(tree, idx, "*", "*", ctx),
        "del" | "s" | "strike" => render_wrap(tree, idx, "~~", "~~", ctx),
        // Block-y wrappers — just walk children.
        "article" | "main" | "section" | "div" | "body" | "#document" | "header" | "nav"
        | "footer" | "aside" | "span" | "html" | "details" | "summary" | "figure" | "figcaption" => {
            if !elem.own_text.trim().is_empty() {
                push_text(&elem.own_text, ctx);
            }
            for &c in &elem.children {
                render_node(tree, c, ctx);
            }
        }
        _ => {
            // Unknown element — treat as inline container.
            if !elem.own_text.trim().is_empty() {
                push_text(&elem.own_text, ctx);
            }
            for &c in &elem.children {
                render_node(tree, c, ctx);
            }
        }
    }
}

fn render_heading(tree: &Tree, idx: usize, level: usize, ctx: &mut RenderCtx<'_>) {
    block_break(ctx);
    ctx.out.push_str(&"#".repeat(level));
    ctx.out.push(' ');
    render_inline_children(tree, idx, ctx);
    ctx.out.push_str("\n\n");
    ctx.last_was_block = true;
}

fn render_block_paragraph(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    block_break(ctx);
    let elem = tree.get(idx);
    if !elem.own_text.trim().is_empty() {
        push_text(&elem.own_text, ctx);
    }
    for &c in &elem.children {
        render_node(tree, c, ctx);
    }
    ctx.out.push_str("\n\n");
    ctx.last_was_block = true;
}

fn render_blockquote(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    block_break(ctx);
    let mut inner = String::new();
    {
        let mut sub_ctx = RenderCtx {
            out: std::mem::take(&mut inner),
            list_stack: Vec::new(),
            in_pre: ctx.in_pre,
            in_link: ctx.in_link,
            skip: ctx.skip,
            options: ctx.options,
            text_chars: 0,
            last_was_block: true,
        };
        let elem = tree.get(idx);
        if !elem.own_text.trim().is_empty() {
            push_text(&elem.own_text, &mut sub_ctx);
        }
        for &c in &elem.children {
            render_node(tree, c, &mut sub_ctx);
        }
        inner = sub_ctx.out;
        ctx.text_chars += sub_ctx.text_chars;
    }
    for line in inner.trim_end().split('\n') {
        ctx.out.push_str("> ");
        ctx.out.push_str(line);
        ctx.out.push('\n');
    }
    ctx.out.push('\n');
    ctx.last_was_block = true;
}

fn render_pre(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    block_break(ctx);
    // Detect language hint from class.
    let elem = tree.get(idx);
    let mut lang = String::new();
    if let Some(class) = elem.attr("class") {
        for token in class.split_whitespace() {
            if let Some(rest) = token.strip_prefix("language-") {
                lang = rest.to_string();
                break;
            }
            if let Some(rest) = token.strip_prefix("lang-") {
                lang = rest.to_string();
                break;
            }
        }
    }
    // If there's a single <code> child, prefer its class.
    if lang.is_empty() {
        for &c in &elem.children {
            let child = tree.get(c);
            if child.tag == "code" {
                if let Some(class) = child.attr("class") {
                    for token in class.split_whitespace() {
                        if let Some(rest) = token.strip_prefix("language-") {
                            lang = rest.to_string();
                            break;
                        }
                        if let Some(rest) = token.strip_prefix("lang-") {
                            lang = rest.to_string();
                            break;
                        }
                    }
                }
            }
        }
    }
    ctx.out.push_str("```");
    if !lang.is_empty() {
        ctx.out.push_str(&lang);
    }
    ctx.out.push('\n');
    let prev_in_pre = ctx.in_pre;
    ctx.in_pre = true;
    let mut text_buf = String::new();
    collect_text(tree, idx, &mut text_buf);
    ctx.in_pre = prev_in_pre;
    let trimmed = text_buf.trim_matches('\n');
    ctx.out.push_str(trimmed);
    if !trimmed.ends_with('\n') {
        ctx.out.push('\n');
    }
    ctx.out.push_str("```\n\n");
    ctx.text_chars += trimmed.chars().count();
    ctx.last_was_block = true;
}

fn render_code_inline(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    let mut text = String::new();
    collect_text(tree, idx, &mut text);
    if text.is_empty() {
        return;
    }
    ctx.out.push('`');
    ctx.out.push_str(text.trim());
    ctx.out.push('`');
    ctx.text_chars += text.trim().chars().count();
    ctx.last_was_block = false;
}

fn render_list(tree: &Tree, idx: usize, ordered: bool, ctx: &mut RenderCtx<'_>) {
    block_break(ctx);
    ctx.list_stack.push(ListFrame {
        ordered,
        counter: 1,
    });
    let elem = tree.get(idx);
    for &c in &elem.children {
        let child = tree.get(c);
        if child.tag == "li" {
            render_list_item(tree, c, ctx);
        } else if child.tag != "_dropped_" {
            render_node(tree, c, ctx);
        }
    }
    ctx.list_stack.pop();
    ctx.out.push('\n');
    ctx.last_was_block = true;
}

fn render_list_item(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    // Determine bullet from the innermost list frame.
    let (depth, marker) = {
        let n = ctx.list_stack.len();
        if let Some(frame) = ctx.list_stack.last_mut() {
            let m = if frame.ordered {
                let s = format!("{}. ", frame.counter);
                frame.counter += 1;
                s
            } else {
                "- ".to_string()
            };
            (n.saturating_sub(1), m)
        } else {
            (0, "- ".to_string())
        }
    };
    let indent = "  ".repeat(depth);
    ctx.out.push_str(&indent);
    ctx.out.push_str(&marker);
    let elem = tree.get(idx);
    if !elem.own_text.trim().is_empty() {
        push_text(&elem.own_text, ctx);
    }
    let mut had_nested_block = false;
    for &c in &elem.children {
        let child = tree.get(c);
        match child.tag.as_str() {
            "ul" | "ol" => {
                ctx.out.push('\n');
                had_nested_block = true;
                render_node(tree, c, ctx);
            }
            "p" => {
                if !child.own_text.trim().is_empty() {
                    push_text(&child.own_text, ctx);
                }
                for &cc in &child.children {
                    render_node(tree, cc, ctx);
                }
                ctx.out.push('\n');
            }
            _ => render_node(tree, c, ctx),
        }
    }
    if !had_nested_block {
        ctx.out.push('\n');
    }
    ctx.last_was_block = false;
}

fn render_table(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    let elem = tree.get(idx);
    // Collect rows (descend through <thead>/<tbody>/<tfoot> implicitly).
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut header_row: Option<Vec<String>> = None;
    let mut stack: Vec<usize> = elem.children.iter().rev().copied().collect();
    while let Some(c) = stack.pop() {
        let n = tree.get(c);
        match n.tag.as_str() {
            "_dropped_" => {}
            "tr" => {
                let mut row: Vec<String> = Vec::new();
                let mut row_is_header = false;
                for &cc in &n.children {
                    let cell = tree.get(cc);
                    if !matches!(cell.tag.as_str(), "td" | "th") {
                        continue;
                    }
                    if cell.tag == "th" {
                        row_is_header = true;
                    }
                    let mut cell_text = String::new();
                    collect_text(tree, cc, &mut cell_text);
                    row.push(escape_table_cell(cell_text.trim()));
                }
                if !row.is_empty() {
                    if row_is_header && header_row.is_none() {
                        header_row = Some(row);
                    } else {
                        rows.push(row);
                    }
                }
            }
            // Recurse into thead/tbody/tfoot
            "thead" | "tbody" | "tfoot" => {
                for &cc in n.children.iter().rev() {
                    stack.push(cc);
                }
            }
            _ => {}
        }
    }
    let header = header_row.unwrap_or_else(|| {
        if let Some(first) = rows.first().cloned() {
            rows.remove(0);
            first
        } else {
            Vec::new()
        }
    });
    if header.is_empty() && rows.is_empty() {
        return;
    }
    block_break(ctx);
    let cols = header
        .len()
        .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));
    let h: Vec<String> = (0..cols)
        .map(|i| header.get(i).cloned().unwrap_or_default())
        .collect();
    ctx.out.push_str("| ");
    ctx.out.push_str(&h.join(" | "));
    ctx.out.push_str(" |\n|");
    for _ in 0..cols {
        ctx.out.push_str(" --- |");
    }
    ctx.out.push('\n');
    for row in &rows {
        let r: Vec<String> = (0..cols)
            .map(|i| row.get(i).cloned().unwrap_or_default())
            .collect();
        ctx.out.push_str("| ");
        ctx.out.push_str(&r.join(" | "));
        ctx.out.push_str(" |\n");
    }
    ctx.out.push('\n');
    ctx.last_was_block = true;
}

fn render_image(elem: &Element, ctx: &mut RenderCtx<'_>) {
    let src = elem.attr("src").or_else(|| elem.attr("data-src"));
    if let Some(src) = src {
        let alt = elem.attr("alt").unwrap_or("");
        ctx.out.push_str("![");
        ctx.out.push_str(alt);
        ctx.out.push_str("](");
        ctx.out.push_str(src);
        ctx.out.push(')');
        ctx.last_was_block = false;
    }
}

fn render_link(tree: &Tree, idx: usize, elem: &Element, ctx: &mut RenderCtx<'_>) {
    let href = elem.attr("href").unwrap_or("");
    let mut text = String::new();
    collect_text(tree, idx, &mut text);
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if ctx.in_link > 0 {
        // Nested links are flattened to plain text to avoid markdown chaos.
        ctx.out.push_str(text);
        return;
    }
    if href.is_empty() {
        ctx.out.push_str(text);
        ctx.text_chars += text.chars().count();
        return;
    }
    ctx.in_link += 1;
    ctx.out.push('[');
    ctx.out.push_str(text);
    ctx.out.push_str("](");
    ctx.out.push_str(href);
    ctx.out.push(')');
    ctx.in_link -= 1;
    ctx.text_chars += text.chars().count();
    ctx.last_was_block = false;
}

fn render_wrap(tree: &Tree, idx: usize, open: &str, close: &str, ctx: &mut RenderCtx<'_>) {
    let mut inner = String::new();
    let elem = tree.get(idx);
    if !elem.own_text.trim().is_empty() {
        inner.push_str(elem.own_text.trim());
    }
    for &c in &elem.children {
        let mut sub = RenderCtx {
            out: std::mem::take(&mut inner),
            list_stack: Vec::new(),
            in_pre: ctx.in_pre,
            in_link: ctx.in_link,
            skip: ctx.skip,
            options: ctx.options,
            text_chars: 0,
            last_was_block: ctx.last_was_block,
        };
        render_node(tree, c, &mut sub);
        ctx.text_chars += sub.text_chars;
        inner = sub.out;
    }
    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return;
    }
    ctx.out.push_str(open);
    ctx.out.push_str(trimmed);
    ctx.out.push_str(close);
    ctx.last_was_block = false;
}

fn render_inline_children(tree: &Tree, idx: usize, ctx: &mut RenderCtx<'_>) {
    let elem = tree.get(idx);
    if !elem.own_text.trim().is_empty() {
        push_text(&elem.own_text, ctx);
    }
    for &c in &elem.children {
        render_node(tree, c, ctx);
    }
}

fn collect_text(tree: &Tree, idx: usize, buf: &mut String) {
    tree.walk_subtree_text(idx, &mut |elem| {
        if elem.tag == "_dropped_" {
            return false;
        }
        if !elem.own_text.is_empty() {
            if !buf.is_empty()
                && !buf.ends_with(char::is_whitespace)
                && !elem.own_text.starts_with(char::is_whitespace)
            {
                buf.push(' ');
            }
            buf.push_str(&elem.own_text);
        }
        true
    });
}

fn push_text(text: &str, ctx: &mut RenderCtx<'_>) {
    let prepared = if ctx.in_pre {
        text.to_string()
    } else {
        collapse_ws(text)
    };
    if prepared.is_empty() {
        return;
    }
    if !ctx.out.ends_with(' ')
        && !ctx.out.ends_with('\n')
        && !ctx.out.is_empty()
        && !prepared.starts_with(' ')
    {
        ctx.out.push(' ');
    }
    ctx.text_chars += prepared.chars().count();
    ctx.out.push_str(&prepared);
    ctx.last_was_block = false;
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn block_break(ctx: &mut RenderCtx<'_>) {
    if ctx.out.is_empty() {
        return;
    }
    if !ctx.out.ends_with("\n\n") {
        if ctx.out.ends_with('\n') {
            ctx.out.push('\n');
        } else {
            ctx.out.push_str("\n\n");
        }
    }
}

fn normalize_whitespace(s: &str) -> String {
    // Collapse 3+ blank lines to 2 and trim trailing spaces on each line.
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.split('\n') {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(trimmed_end);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

fn escape_table_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace_runs() {
        assert_eq!(collapse_ws("  hello   world  "), "hello world");
    }

    #[test]
    fn normalize_collapses_blank_lines() {
        let n = normalize_whitespace("a\n\n\n\nb");
        assert_eq!(n, "a\n\nb");
    }

    #[test]
    fn escape_table_cell_handles_pipes_and_newlines() {
        assert_eq!(escape_table_cell("a|b\nc"), "a\\|b c");
    }
}

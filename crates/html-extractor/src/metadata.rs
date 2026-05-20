//! Metadata extraction from JSON-LD, OpenGraph, Twitter Card, standard meta,
//! and `<title>` / `<html lang>` fallbacks.

use crate::tree::Tree;
use crate::types::Metadata;

/// Run metadata extraction over the parsed (pre-clean) tree.
pub(crate) fn extract(tree: &Tree) -> Metadata {
    let mut md = Metadata::default();

    // 1. Walk into `<head>` once for all meta-style sources.
    if tree.head != usize::MAX {
        collect_head(tree, tree.head, &mut md);
    }

    // 2. Title fallback from <title>.
    if md.title.is_none() && !tree.title_text.is_empty() {
        md.title = Some(strip_title(&tree.title_text));
    }

    // 3. Language fallback from <html lang>.
    if md.language.is_none() {
        if let Some(lang) = &tree.html_lang {
            md.language = Some(lang.clone());
        }
    }

    // 4. JSON-LD walk
    walk_jsonld(tree, &mut md);

    md
}

fn collect_head(tree: &Tree, head: usize, md: &mut Metadata) {
    tree.walk_pre(head, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        match elem.tag.as_str() {
            "meta" => apply_meta(elem, md),
            "link" => apply_link(elem, md),
            "title" if md.title.is_none() => {
                let t = strip_title(&tree.full_text(idx));
                if !t.is_empty() {
                    md.title = Some(t);
                }
            }
            "html" if md.language.is_none() => {
                if let Some(lang) = elem.attr("lang") {
                    md.language = Some(lang.to_string());
                }
            }
            _ => {}
        }
        true
    });
}

fn apply_meta(elem: &crate::tree::Element, md: &mut Metadata) {
    let content = match elem.attr("content") {
        Some(c) if !c.trim().is_empty() => c.trim().to_string(),
        _ => return,
    };
    if let Some(property) = elem.attr("property") {
        let p = property.to_ascii_lowercase();
        match p.as_str() {
            "og:title" => set_if_empty(&mut md.title, &content),
            "og:description" => set_if_empty(&mut md.description, &content),
            "og:site_name" => set_if_empty(&mut md.site_name, &content),
            "og:image" | "og:image:url" | "og:image:secure_url" => {
                set_if_empty(&mut md.image_url, &content)
            }
            "og:url" => set_if_empty(&mut md.canonical_url, &content),
            "article:author" => set_if_empty(&mut md.author, &content),
            "article:published_time" => set_if_empty(&mut md.published_date, &content),
            "article:tag" => md.keywords.push(content.clone()),
            _ => {}
        }
    }
    if let Some(name) = elem.attr("name") {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "twitter:title" => set_if_empty(&mut md.title, &content),
            "twitter:description" => set_if_empty(&mut md.description, &content),
            "twitter:image" | "twitter:image:src" => set_if_empty(&mut md.image_url, &content),
            "twitter:site" | "application-name" => set_if_empty(&mut md.site_name, &content),
            "description" | "dc.description" | "dcterms.description" => {
                set_if_empty(&mut md.description, &content)
            }
            "author" | "byl" | "citation_author" | "dc.creator" => {
                set_if_empty(&mut md.author, &content)
            }
            "keywords" => {
                for tag in content.split(',') {
                    let t = tag.trim();
                    if !t.is_empty() {
                        md.keywords.push(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(itemprop) = elem.attr("itemprop") {
        let i = itemprop.to_ascii_lowercase();
        match i.as_str() {
            "author" => set_if_empty(&mut md.author, &content),
            "description" => set_if_empty(&mut md.description, &content),
            "headline" => set_if_empty(&mut md.title, &content),
            "datepublished" => set_if_empty(&mut md.published_date, &content),
            _ => {}
        }
    }
}

fn apply_link(elem: &crate::tree::Element, md: &mut Metadata) {
    let rel = elem
        .attr("rel")
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if rel.split_whitespace().any(|t| t == "canonical") {
        if let Some(href) = elem.attr("href") {
            set_if_empty(&mut md.canonical_url, href.trim());
        }
    }
}

fn set_if_empty(slot: &mut Option<String>, val: &str) {
    if slot.is_none() && !val.trim().is_empty() {
        *slot = Some(val.trim().to_string());
    }
}

fn strip_title(t: &str) -> String {
    // A surprising number of <title>s look like "Article name — Site name".
    // We keep the full string; downstream consumers can split if they wish.
    t.trim().to_string()
}

fn walk_jsonld(tree: &Tree, md: &mut Metadata) {
    tree.walk_pre(tree.root, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        if elem.tag != "script" {
            return true;
        }
        let ty = elem.attr("type").unwrap_or("");
        if !ty.eq_ignore_ascii_case("application/ld+json") {
            return true;
        }
        let raw = elem.own_text.trim();
        if raw.is_empty() {
            return true;
        }
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) {
            absorb_jsonld(&val, md);
        }
        true
    });
}

fn absorb_jsonld(v: &serde_json::Value, md: &mut Metadata) {
    match v {
        serde_json::Value::Array(items) => {
            for item in items {
                absorb_jsonld(item, md);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(graph) = map.get("@graph") {
                absorb_jsonld(graph, md);
            }
            if let Some(serde_json::Value::String(t)) =
                map.get("headline").or_else(|| map.get("name"))
            {
                set_if_empty(&mut md.title, t);
            }
            if let Some(serde_json::Value::String(d)) = map.get("description") {
                set_if_empty(&mut md.description, d);
            }
            if let Some(serde_json::Value::String(date)) = map.get("datePublished") {
                set_if_empty(&mut md.published_date, date);
            }
            if let Some(author) = map.get("author") {
                if let Some(name) = jsonld_name(author) {
                    set_if_empty(&mut md.author, &name);
                }
            }
            if let Some(publisher) = map.get("publisher") {
                if let Some(name) = jsonld_name(publisher) {
                    set_if_empty(&mut md.site_name, &name);
                }
            }
            if let Some(img) = map.get("image") {
                if let Some(url) = jsonld_url(img) {
                    set_if_empty(&mut md.image_url, &url);
                }
            }
            if let Some(serde_json::Value::String(u)) = map.get("url") {
                set_if_empty(&mut md.canonical_url, u);
            }
            if let Some(serde_json::Value::String(lang)) = map.get("inLanguage") {
                set_if_empty(&mut md.language, lang);
            }
            // schema.org @type — used by the page-type classifier. Can be a
            // single string, an array of strings, or absent.
            if md.schema_type.is_none() {
                if let Some(t) = map.get("@type") {
                    match t {
                        serde_json::Value::String(s) => md.schema_type = Some(s.clone()),
                        serde_json::Value::Array(items) => {
                            if let Some(serde_json::Value::String(s)) = items.first() {
                                md.schema_type = Some(s.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
}

fn jsonld_name(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(m) => {
            if let Some(serde_json::Value::String(s)) = m.get("name") {
                Some(s.clone())
            } else {
                None
            }
        }
        serde_json::Value::Array(items) => items.iter().find_map(jsonld_name),
        _ => None,
    }
}

fn jsonld_url(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(m) => {
            if let Some(serde_json::Value::String(s)) = m.get("url") {
                Some(s.clone())
            } else {
                None
            }
        }
        serde_json::Value::Array(items) => items.iter().find_map(jsonld_url),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn extracts_basic_meta_and_og() {
        let html = r#"<html lang="es">
            <head>
                <title>Hi</title>
                <meta name="author" content="A. Writer">
                <meta name="description" content="Desc">
                <meta property="og:site_name" content="Site">
                <meta property="og:image" content="https://x/a.png">
                <link rel="canonical" href="https://x/canon">
            </head><body><p>x</p></body></html>"#;
        let tree = parse(html).unwrap();
        let md = extract(&tree);
        assert_eq!(md.author.as_deref(), Some("A. Writer"));
        assert_eq!(md.description.as_deref(), Some("Desc"));
        assert_eq!(md.site_name.as_deref(), Some("Site"));
        assert_eq!(md.image_url.as_deref(), Some("https://x/a.png"));
        assert_eq!(md.canonical_url.as_deref(), Some("https://x/canon"));
        assert_eq!(md.language.as_deref(), Some("es"));
        assert_eq!(md.title.as_deref(), Some("Hi"));
    }

    #[test]
    fn jsonld_article_pulls_headline_and_date() {
        let html = r#"<html><head><script type="application/ld+json">
            {"@type":"NewsArticle","headline":"Big news","datePublished":"2024-01-02","author":{"name":"J. Reporter"}}
        </script></head><body><p>x</p></body></html>"#;
        let tree = parse(html).unwrap();
        let md = extract(&tree);
        assert_eq!(md.title.as_deref(), Some("Big news"));
        assert_eq!(md.published_date.as_deref(), Some("2024-01-02"));
        assert_eq!(md.author.as_deref(), Some("J. Reporter"));
    }

    #[test]
    fn meta_keywords_split() {
        let html = r#"<html><head><meta name="keywords" content="alpha, beta , gamma"></head><body></body></html>"#;
        let tree = parse(html).unwrap();
        let md = extract(&tree);
        assert_eq!(md.keywords, vec!["alpha", "beta", "gamma"]);
    }
}

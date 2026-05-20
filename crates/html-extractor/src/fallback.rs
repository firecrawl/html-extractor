//! Stage 4 — fallback chain.
//!
//! Justext-style paragraph classification, readability-style scored
//! aggregation, and a raw-body last resort. We don't import external
//! justext / readability crates; the logic here captures the algorithm
//! described in ALGORITHM.md Stage 4.

use crate::tree::Tree;
use crate::types::ExtractOptions;

/// Run the fallback chain. Returns `(chosen_subtree, quality)`.
pub(crate) fn fallback(tree: &Tree, _options: &ExtractOptions) -> (Option<usize>, f32) {
    if tree.body == usize::MAX {
        return (None, 0.0);
    }
    // 4a. Justext-style: walk every block element, classify as good/bad by
    // text length + stop-word presence + link density, and pick the parent
    // that contains the most "good" content.
    if let Some(idx) = justext_pick(tree) {
        return (Some(idx), 0.4);
    }
    // 4b. Readability-style: score every <p>, propagate to parent, pick top.
    if let Some(idx) = readability_pick(tree) {
        return (Some(idx), 0.3);
    }
    // 4c. Raw body: extraction_quality stays low. Return body itself.
    (Some(tree.body), 0.15)
}

fn justext_pick(tree: &Tree) -> Option<usize> {
    // Per-paragraph classification.
    let mut good_parents: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::new();
    tree.walk_pre(tree.body, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        if !matches!(
            elem.tag.as_str(),
            "p" | "li" | "blockquote" | "pre" | "section"
        ) {
            return true;
        }
        let txt = tree.full_text(idx);
        let trimmed = txt.trim();
        if trimmed.len() < 40 {
            return true;
        }
        // Link density check
        let link_chars: usize = subtree_link_chars(tree, idx);
        let total = trimmed.chars().count();
        if total == 0 {
            return true;
        }
        let link_ratio = link_chars as f32 / total as f32;
        if link_ratio > 0.5 {
            return true;
        }
        if !has_stop_words(trimmed) && total < 200 {
            return true;
        }
        // Mark the nearest "block" ancestor as receiving a good paragraph.
        let mut ancestor = elem.parent;
        let mut hops = 0;
        while ancestor != usize::MAX && hops < 6 {
            let a = tree.get(ancestor);
            if matches!(
                a.tag.as_str(),
                "div" | "article" | "section" | "main" | "body" | "td"
            ) {
                *good_parents.entry(ancestor).or_insert(0) += total;
                break;
            }
            ancestor = a.parent;
            hops += 1;
        }
        true
    });
    good_parents
        .into_iter()
        .max_by_key(|&(_, n)| n)
        .map(|(idx, _)| idx)
}

fn readability_pick(tree: &Tree) -> Option<usize> {
    let mut scores: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
    tree.walk_pre(tree.body, |idx| {
        let elem = tree.get(idx);
        if elem.tag == "_dropped_" {
            return false;
        }
        if elem.tag == "p" {
            let text = elem.own_text.trim();
            if text.len() < 25 {
                return true;
            }
            let commas = text.matches(',').count() as f32;
            let len_score = (text.chars().count() as f32 / 100.0).min(3.0);
            let base = 1.0 + commas + len_score;
            // Distribute to parent and grandparent.
            let mut up = elem.parent;
            let mut decay = 1.0;
            for _ in 0..3 {
                if up == usize::MAX {
                    break;
                }
                *scores.entry(up).or_insert(0.0) += base * decay;
                up = tree.get(up).parent;
                decay *= 0.5;
            }
        }
        true
    });
    scores
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(idx, _)| idx)
}

fn subtree_link_chars(tree: &Tree, idx: usize) -> usize {
    let mut n = 0;
    tree.walk_subtree_text(idx, &mut |elem| {
        if elem.tag == "a" {
            n += elem.own_text.chars().count();
        }
        true
    });
    n
}

const STOP_WORDS: &[&str] = &[
    // English
    " the ", " and ", " of ", " to ", " in ", " is ", " that ", " for ", " with ", " on ", " was ",
    " are ", " be ", " not ", " from ", " by ", " this ", " it ", " an ", " as ", " at ", " or ",
    " have ", " but ", " has ", " they ", " we ", " their ", " its ", " more ", " also ", " all ",
    " can ", " had ", " will ", " would ", " been ", " one ", " out ", " when ", " which ",
    " who ", " these ", " those ", // German
    " der ", " die ", " das ", " und ", " ist ", " nicht ", " auf ", " mit ", " für ", " von ",
    " zu ", " aus ", " bei ", " nach ", " im ", " am ", " eine ", " einen ", " einem ", " einer ",
    " den ", " als ", " auch ", " werden ", " wird ", " wurde ", " sich ", " sind ", " war ",
    " noch ", " nur ", " wenn ", " man ", " sie ", " es ", " ein ", " des ", " dem ", " durch ",
    // Spanish
    " el ", " la ", " los ", " las ", " que ", " de ", " del ", " en ", " un ", " una ", " unos ",
    " unas ", " por ", " con ", " su ", " sus ", " como ", " es ", " son ", " para ", " no ",
    " se ", " lo ", " ha ", " había ", " sin ", " sobre ", // French
    " le ", " la ", " les ", " un ", " une ", " des ", " et ", " ou ", " de ", " du ", " dans ",
    " avec ", " pour ", " sur ", " que ", " qui ", " est ", " sont ", " ne ", " ce ", " cette ",
    " ces ", " son ", " ses ", " au ", " aux ", " par ", " pas ", // Italian
    " il ", " lo ", " gli ", " le ", " di ", " da ", " in ", " con ", " su ", " per ", " tra ",
    " fra ", " e ", " è ", " sono ", " che ", " un ", " una ", " uno ", " del ", " della ",
    " dei ", " delle ", " si ", " ma ", " non ", // Portuguese
    " o ", " a ", " os ", " as ", " de ", " da ", " do ", " das ", " dos ", " e ", " em ", " com ",
    " para ", " que ", " é ", " são ", " um ", " uma ", " uns ", " umas ", " no ", " na ", " nos ",
    " nas ", " por ", " se ", // Polish (corpus has some Polish pages)
    " i ", " w ", " na ", " z ", " do ", " że ", " jest ", " to ", " nie ", " jak ", " co ",
    " się ",
];

fn has_stop_words(text: &str) -> bool {
    let lower = format!(" {} ", text.to_lowercase());
    STOP_WORDS
        .iter()
        .filter(|w| lower.contains(*w))
        .take(2)
        .count()
        >= 1
}

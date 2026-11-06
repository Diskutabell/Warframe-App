//! Turning tesseract's words into reward names.
//!
//! Most rules below exist because some reward screen was misread without them. It is a port of
//! the Python matcher (thresholds, windows, suppression and tie-breaks).

use std::collections::{BTreeSet, HashSet};

use indexmap::IndexMap;

use crate::difflib::{SequenceMatcher, get_close_matches, py_round};

/// One word from tesseract's TSV output.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub height: i32,
}

/// A reward name found on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct RewardMatch {
    pub name: String,
    pub uid: String,
    pub score: f64,
}

/// The names the matcher looks for, indexed by their first word.
pub struct Names {
    entries: Vec<(String, String)>,
    tokens: Vec<Vec<String>>,
    first: IndexMap<String, Vec<usize>>,
}

impl Names {
    /// `names` maps display name -> uid, in catalog order.
    pub fn new(names: &IndexMap<String, String>) -> Names {
        let mut entries = Vec::with_capacity(names.len());
        let mut tokens = Vec::with_capacity(names.len());
        let mut first: IndexMap<String, Vec<usize>> = IndexMap::new();
        for (i, (name, uid)) in names.iter().enumerate() {
            let toks = norm(name);
            if let Some(head) = toks.first() {
                first.entry(head.clone()).or_default().push(i);
            }
            entries.push((name.clone(), uid.clone()));
            tokens.push(toks);
        }
        Names { entries, tokens, first }
    }
}

/// Lower-case, everything but a-z, 0-9 and spaces becomes a space, split on whitespace.
pub fn norm(s: &str) -> Vec<String> {
    let cleaned: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' { c } else { ' ' })
        .collect();
    cleaned.split_whitespace().map(str::to_owned).collect()
}

/// Groups words into reward cards: cards sit side by side, names wrap onto 1-3 lines. Returns
/// each column's words in reading order, left to right.
///
/// The grouping only has to keep one card's words together and in order - it is not trusted to
/// separate the cards, because scenery words routinely bridge the gap between two of them.
pub fn columns(words: &[Word]) -> Vec<Vec<&Word>> {
    if words.is_empty() {
        return Vec::new();
    }
    let mut heights: Vec<i32> = words.iter().map(|w| w.height).collect();
    heights.sort_unstable();
    let med_h = heights[words.len() / 2];
    let gap = med_h as f64 * 1.2;
    let mut by_left: Vec<&Word> = words.iter().collect();
    by_left.sort_by_key(|w| w.left);

    struct Col<'a> {
        left: i32,
        right: i32,
        words: Vec<&'a Word>,
    }
    let mut cols: Vec<Col> = Vec::new();
    for w in by_left {
        let hit =
            cols.iter_mut().find(|c| w.left as f64 <= c.right as f64 + gap && w.right as f64 >= c.left as f64 - gap);
        match hit {
            Some(c) => {
                c.left = c.left.min(w.left);
                c.right = c.right.max(w.right);
                c.words.push(w);
            }
            None => cols.push(Col { left: w.left, right: w.right, words: vec![w] }),
        }
    }
    cols.sort_by_key(|c| c.left);
    let line = |w: &Word| (w.top as f64 / med_h.max(1) as f64).round_ties_even() as i64;
    cols.into_iter()
        .map(|mut c| {
            c.words.sort_by_key(|w| (line(w), w.left));
            c.words
        })
        .collect()
}

/// A name found in one run of words, with the left edge of its first word.
#[derive(Clone, Debug)]
struct Hit {
    score: f64,
    entry: usize,
    x: i32,
}

/// Every reward name in one column of words.
///
/// A column is a card's worth of OCR - the name plus scenery noise - so comparing it end to end
/// with a clean name scores far below threshold. Slide a window of the name's own length (+-1
/// token) instead and keep the best fit.
///
/// Several names can live in one column: the gap-based grouping merges two cards as soon as a
/// stray glyph lands between them, which is the normal case on a 4-reward screen. So collect hits
/// for every candidate and take them greedily, best first, with no two sharing a word.
///
/// Keep the best window of every *place* a name turns up, not just its single best window.
/// Sibling part names differ by one token, so a name's top-scoring window is often on another
/// card: "Ivara Prime Chassis Blueprint" scored 0.881 against the Yareli card and only 0.64 on
/// its own garbled text, and with one window per name it vanished from a screen it was on.
fn find_names(words: &[&Word], names: &Names) -> Vec<Hit> {
    let mut toks: Vec<String> = Vec::new();
    let mut boxes: Vec<&Word> = Vec::new();
    for w in words {
        for t in norm(&w.text) {
            toks.push(t);
            boxes.push(w);
        }
    }
    if toks.is_empty() {
        return Vec::new();
    }
    let mut cands: BTreeSet<usize> = BTreeSet::new();
    let unique: HashSet<&str> = toks.iter().map(String::as_str).collect();
    for t in unique {
        for head in get_close_matches(t, names.first.keys().map(String::as_str), 3, 0.7) {
            cands.extend(&names.first[head]);
        }
    }

    let mut hits: Vec<(f64, usize, usize, usize)> = Vec::new();
    for &e in &cands {
        let nt = &names.tokens[e];
        if nt.is_empty() {
            continue;
        }
        let name = &names.entries[e].0;
        // Short names (the Requiem mod "Ris") fuzzy-match stray text easily; demand near-exact.
        let need = if name.chars().count() <= 6 { 0.95 } else { 0.72 };
        let k = nt.len();
        let mut sm = SequenceMatcher::new("", &nt.join(" "));
        let mut found: Vec<(f64, usize, usize)> = Vec::new();
        let widths: BTreeSet<usize> = [(k - 1).max(1), k, k + 1].into_iter().collect();
        for width in widths {
            let starts = (toks.len() as isize - width as isize + 1).max(1) as usize;
            for i in 0..starts {
                let j = (i + width).min(toks.len());
                sm.set_seq1(&toks[i..j].join(" "));
                if sm.real_quick_ratio() < need || sm.quick_ratio() < need {
                    continue;
                }
                let r = sm.ratio();
                if r >= need {
                    found.push((r, i, j));
                }
            }
        }
        // One window per place: best first, then drop every window overlapping a kept one.
        found.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut kept: Vec<(f64, usize, usize)> = Vec::new();
        for (r, i, j) in found {
            if kept.iter().all(|&(_, a, b)| j <= a || i >= b) {
                kept.push((r, i, j));
            }
        }
        hits.extend(kept.into_iter().map(|(r, i, j)| (r, e, i, j)));
    }

    // Scores within a hundredth are the same score as far as tesseract is concerned, and the
    // longer name is the specific one: "Gyre Prime Systems Blueprint" must beat its sibling
    // "Gyre Prime Blueprint" on the same text, not lose to it by 0.003.
    hits.sort_by(|a, b| {
        let (na, nb) = (&names.entries[a.1].0, &names.entries[b.1].0);
        py_round(b.0, 2)
            .total_cmp(&py_round(a.0, 2))
            .then_with(|| nb.chars().count().cmp(&na.chars().count()))
            .then_with(|| na.cmp(nb))
    });
    let mut taken: HashSet<usize> = HashSet::new();
    let mut out = Vec::new();
    for (score, e, i, j) in hits {
        if (i..j).any(|n| taken.contains(&n)) {
            continue;
        }
        taken.extend(i..j);
        let x = (i..j).map(|n| boxes[n].left).min().expect("non-empty span");
        out.push(Hit { score, entry: e, x });
    }
    out
}

/// The rewards on screen, given the words tesseract read from each preprocessed variant.
///
/// Each variant is scored on its best four hits (a screen holds at most four rewards), so a
/// variant that reads one name perfectly does not beat one that reads all four. Returns the
/// matches left to right, and the per-variant text for the record.
pub fn pick_rewards(variants: &[Vec<Word>], names: &Names) -> (Vec<RewardMatch>, String) {
    let mut best: Vec<Hit> = Vec::new();
    let mut best_total = -1.0;
    let mut texts = Vec::new();
    for words in variants {
        let cols = columns(words);
        texts.push(
            cols.iter()
                .map(|c| c.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join(" | "),
        );
        let mut picked: Vec<Hit> = cols.iter().flat_map(|c| find_names(c, names)).collect();
        picked.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut rows: Vec<Hit> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::new();
        for h in picked {
            // A repeated name is kept - two players can be handed the same part - but it has to
            // be convincing. A weak repeat is the matcher reaching for a name it already used:
            // on an occluded card it offered a second "Forma Blueprint" at 0.72.
            if seen.contains(&h.entry) && h.score < 0.9 {
                continue;
            }
            seen.insert(h.entry);
            rows.push(h);
            if rows.len() == 4 {
                break;
            }
        }
        let total: f64 = rows.iter().map(|h| h.score).sum();
        if total > best_total {
            best = rows;
            best_total = total;
        }
    }
    // Always left to right, so the overlay lines up with the cards on screen.
    best.sort_by_key(|h| h.x);
    let matches = best
        .into_iter()
        .map(|h| {
            let (name, uid) = &names.entries[h.entry];
            RewardMatch { name: name.clone(), uid: uid.clone(), score: py_round(h.score, 3) }
        })
        .collect();
    (matches, texts.join("\n"))
}

/// Fallback when the column matcher finds nothing: fuzzy-match names anywhere in free text.
pub fn match_rewards(text: &str, names: &Names, limit: usize) -> Vec<RewardMatch> {
    let words = norm(text);
    if words.is_empty() {
        return Vec::new();
    }
    let joined = words.join(" ");
    let first_words: HashSet<&str> = words.iter().map(String::as_str).collect();
    let mut results: Vec<RewardMatch> = Vec::new();
    for (e, (name, uid)) in names.entries.iter().enumerate() {
        let toks = &names.tokens[e];
        let Some(head) = toks.first() else { continue };
        if !first_words.contains(head.as_str())
            && get_close_matches(head, first_words.iter().copied(), 1, 0.75).is_empty()
        {
            continue;
        }
        let n = toks.len();
        let target = toks.join(" ");
        let mut sm = SequenceMatcher::new("", &target);
        let mut best: f64 = 0.0;
        if words.len() >= n {
            for i in 0..=words.len() - n {
                sm.set_seq1(&words[i..i + n].join(" "));
                best = best.max(sm.ratio());
            }
        }
        if joined.contains(&target) {
            best = best.max(0.99);
        }
        let need = if name.chars().count() <= 6 { 0.95 } else { 0.78 };
        if best >= need {
            results.push(RewardMatch { name: name.clone(), uid: uid.clone(), score: py_round(best, 3) });
        }
    }
    results
        .sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| b.name.chars().count().cmp(&a.name.chars().count())));
    let mut picked: Vec<RewardMatch> = Vec::new();
    for r in results {
        if picked.iter().any(|o| r.name.contains(&o.name) || o.name.contains(&r.name)) && r.score < 0.95 {
            continue;
        }
        picked.push(r);
        if picked.len() >= limit {
            break;
        }
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Names {
        Names::new(&list.iter().map(|n| (n.to_string(), format!("/uid/{n}"))).collect())
    }

    fn word(t: &str, left: i32, top: i32) -> Word {
        Word { text: t.into(), left, right: left + 10 * t.len() as i32, top, height: 20 }
    }

    #[test]
    fn norm_drops_punctuation_and_case() {
        assert_eq!(norm("Paris Prime String!"), vec!["paris", "prime", "string"]);
        assert_eq!(norm("  "), Vec::<String>::new());
    }

    #[test]
    fn four_names_in_one_merged_column_all_match() {
        // Every card's name landed in one column.
        let n = names(&[
            "Gyre Prime Systems Blueprint",
            "Gyre Prime Blueprint",
            "Dual Zoren Prime Handle",
            "Paris Prime String",
            "Braton Prime Receiver",
        ]);
        let text = "Gyre Pri stems Blueprlnt Dual Zoren Prime Handle Paris Prime Strmg Braton Prime Recelver";
        // Overlapping boxes, so every word lands in one column as on the screen this came from.
        let words: Vec<Word> = text
            .split(' ')
            .enumerate()
            .map(|(i, t)| Word { text: t.into(), left: i as i32 * 40, right: i as i32 * 40 + 60, top: 0, height: 20 })
            .collect();
        let (found, _) = pick_rewards(&[words], &n);
        let got: Vec<(&str, f64)> = found.iter().map(|m| (m.name.as_str(), m.score)).collect();
        // The scores the Python matcher gives for the same input.
        assert_eq!(
            got,
            [
                ("Gyre Prime Systems Blueprint", 0.885),
                ("Dual Zoren Prime Handle", 1.0),
                ("Paris Prime String", 0.914),
                ("Braton Prime Receiver", 0.952)
            ]
        );
    }

    #[test]
    fn stray_short_tokens_match_nothing() {
        let n = names(&["Ris", "Forma Blueprint"]);
        let words = vec![word("IME", 0, 0), word("PR", 50, 0), word("AV", 100, 0), word("S", 150, 0)];
        assert!(pick_rewards(&[words], &n).0.is_empty());
    }
}

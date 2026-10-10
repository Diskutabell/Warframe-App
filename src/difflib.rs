//! A faithful port of the parts of Python's `difflib` the OCR matcher uses.
//!
//! The matcher's thresholds (0.72, 0.95, 0.9 for repeats) were tuned by hand against real reward
//! screens using `difflib.SequenceMatcher.ratio()`. A different similarity measure (Levenshtein,
//! Jaro, rapidfuzz's Indel ratio) scores the same strings differently and would silently move
//! every one of those thresholds, so this is Ratcliff/Obershelp exactly as CPython implements it,
//! tie-breaking included.

use std::collections::{HashMap, HashSet};

pub struct SequenceMatcher {
    a: Vec<char>,
    b: Vec<char>,
    b2j: HashMap<char, Vec<usize>>,
    full_b_count: Option<HashMap<char, usize>>,
    matches: Option<usize>,
}

impl SequenceMatcher {
    pub fn new(a: &str, b: &str) -> SequenceMatcher {
        let mut sm = SequenceMatcher {
            a: a.chars().collect(),
            b: Vec::new(),
            b2j: HashMap::new(),
            full_b_count: None,
            matches: None,
        };
        sm.set_seq2(b);
        sm
    }

    /// Replaces `a`, keeping everything computed about `b` - the cheap way to compare many
    /// candidates against one fixed string.
    pub fn set_seq1(&mut self, a: &str) {
        self.a.clear();
        self.a.extend(a.chars());
        self.matches = None;
    }

    pub fn set_seq2(&mut self, b: &str) {
        self.b = b.chars().collect();
        self.matches = None;
        self.full_b_count = None;
        self.chain_b();
    }

    fn chain_b(&mut self) {
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (i, &c) in self.b.iter().enumerate() {
            b2j.entry(c).or_default().push(i);
        }
        // CPython's "autojunk": in sequences of 200+ items, anything making up more than 1% of
        // them is dropped from the index (it is still matched when extending a match).
        let n = self.b.len();
        if n >= 200 {
            let ntest = n / 100 + 1;
            let popular: HashSet<char> = b2j.iter().filter(|(_, idx)| idx.len() > ntest).map(|(&c, _)| c).collect();
            for c in popular {
                b2j.remove(&c);
            }
        }
        self.b2j = b2j;
    }

    /// Longest matching block in a[alo..ahi] and b[blo..bhi], as CPython picks it: the earliest
    /// start in `a`, and for that the earliest start in `b`.
    fn find_longest_match(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (a, b) = (&self.a, &self.b);
        let (mut best_i, mut best_j, mut best_size) = (alo, blo, 0usize);
        // j2len[j + 1] is the length of the match ending at a[i - 1], b[j]; index 0 is a guard.
        let mut j2len = vec![0usize; b.len() + 1];
        let mut new_j2len = vec![0usize; b.len() + 1];
        let mut touched: Vec<usize> = Vec::new();
        let mut new_touched: Vec<usize> = Vec::new();
        for (i, c) in a.iter().enumerate().take(ahi).skip(alo) {
            if let Some(js) = self.b2j.get(c) {
                for &j in js {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j2len[j] + 1;
                    new_j2len[j + 1] = k;
                    new_touched.push(j + 1);
                    if k > best_size {
                        best_i = i + 1 - k;
                        best_j = j + 1 - k;
                        best_size = k;
                    }
                }
            }
            for &t in &touched {
                j2len[t] = 0;
            }
            std::mem::swap(&mut j2len, &mut new_j2len);
            std::mem::swap(&mut touched, &mut new_touched);
            new_touched.clear();
        }
        // Extend with elements the index left out (the "popular" ones). There is no junk
        // predicate here, so the second pair of CPython's loops never applies.
        while best_i > alo && best_j > blo && a[best_i - 1] == b[best_j - 1] {
            best_i -= 1;
            best_j -= 1;
            best_size += 1;
        }
        while best_i + best_size < ahi && best_j + best_size < bhi && a[best_i + best_size] == b[best_j + best_size] {
            best_size += 1;
        }
        (best_i, best_j, best_size)
    }

    fn matching_characters(&mut self) -> usize {
        if let Some(m) = self.matches {
            return m;
        }
        let mut total = 0;
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.find_longest_match(alo, ahi, blo, bhi);
            if k > 0 {
                total += k;
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        self.matches = Some(total);
        total
    }

    pub fn ratio(&mut self) -> f64 {
        let m = self.matching_characters();
        calculate_ratio(m, self.a.len() + self.b.len())
    }

    /// Upper bound on `ratio()`: shared characters, ignoring order.
    pub fn quick_ratio(&mut self) -> f64 {
        if self.full_b_count.is_none() {
            let mut counts = HashMap::new();
            for &c in &self.b {
                *counts.entry(c).or_insert(0usize) += 1;
            }
            self.full_b_count = Some(counts);
        }
        let full = self.full_b_count.as_ref().expect("just filled");
        let mut avail: HashMap<char, isize> = HashMap::new();
        let mut matches = 0;
        for &c in &self.a {
            let numb = *avail.get(&c).unwrap_or(&(full.get(&c).copied().unwrap_or(0) as isize));
            avail.insert(c, numb - 1);
            if numb > 0 {
                matches += 1;
            }
        }
        calculate_ratio(matches, self.a.len() + self.b.len())
    }

    /// Cheapest upper bound on `ratio()`: the shorter length.
    pub fn real_quick_ratio(&self) -> f64 {
        let (la, lb) = (self.a.len(), self.b.len());
        calculate_ratio(la.min(lb), la + lb)
    }
}

fn calculate_ratio(matches: usize, length: usize) -> f64 {
    if length == 0 { 1.0 } else { 2.0 * matches as f64 / length as f64 }
}

/// `difflib.get_close_matches`: the best `n` possibilities scoring at least `cutoff` against
/// `word`, best first. Equal scores are ordered by the possibility itself, descending, which is
/// what CPython's `heapq.nlargest` over `(score, x)` tuples does.
pub fn get_close_matches<'a, I>(word: &str, possibilities: I, n: usize, cutoff: f64) -> Vec<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut sm = SequenceMatcher::new("", word);
    let mut scored: Vec<(f64, &'a str)> = Vec::new();
    for x in possibilities {
        sm.set_seq1(x);
        if sm.real_quick_ratio() >= cutoff && sm.quick_ratio() >= cutoff {
            let r = sm.ratio();
            if r >= cutoff {
                scored.push((r, x));
            }
        }
    }
    scored.sort_by(|p, q| q.0.total_cmp(&p.0).then_with(|| q.1.cmp(p.1)));
    scored.truncate(n);
    scored.into_iter().map(|(_, x)| x).collect()
}

/// Python's `round(x, digits)` for floats: correctly rounded, ties to even on the exact binary
/// value. The matcher sorts on `round(score, 2)`, so near-ties must break the same way.
pub fn py_round(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ratios_match_cpython() {
        // Values from CPython 3.14's difflib.
        let mut sm = SequenceMatcher::new("abcd", "bcde");
        assert_eq!(sm.ratio(), 0.75);
        let mut sm = SequenceMatcher::new("gyre pri stems blueprlnt", "gyre prime systems blueprint");
        assert_eq!(sm.ratio(), 0.8846153846153846);
        assert_eq!(SequenceMatcher::new("", "").real_quick_ratio(), 1.0);
    }

    #[test]
    fn close_matches_order_like_nlargest() {
        let words = ["ape", "apple", "peach", "puppy"];
        assert_eq!(get_close_matches("appel", words, 3, 0.6), vec!["apple", "ape"]);
    }

    #[test]
    fn rounding_is_ties_to_even_like_python() {
        assert_eq!(py_round(0.125, 2), 0.12);
        assert_eq!(py_round(0.375, 2), 0.38);
        assert_eq!(py_round(0.8846153846153846, 3), 0.885);
        assert_eq!(py_round(2.675, 2), 2.67); // 2.675 is really 2.67499999...
    }
}

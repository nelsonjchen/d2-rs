// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2015 The Go Authors. All rights reserved.
// Rust translation of golang.org/x/text v0.41.0 unicode/bidi (bidi.go
// Paragraph.SetString/Order/calculateOrdering, core.go and bracket.go) as
// go-text's Segmenter uses it. The translation keeps x/text's observable
// behavior, including two quirks that differ from UAX #9: brackets pair only
// when the opening and closing runes are equal (matchOpener compares the
// stored runes themselves, so N0 never applies to ordinary pairs), and
// `i = runEnd` inside Go range loops does not skip iterations.

use crate::shaping_tables::BIDI;

type Class = u8;
type Level = i8;

const L: Class = 0;
const R: Class = 1;
const EN: Class = 2;
const ES: Class = 3;
const ET: Class = 4;
const AN: Class = 5;
const CS: Class = 6;
const B: Class = 7;
const S: Class = 8;
const WS: Class = 9;
const ON: Class = 10;
const BN: Class = 11;
const NSM: Class = 12;
const AL: Class = 13;
const LRO: Class = 16;
const RLO: Class = 17;
const LRE: Class = 18;
const RLE: Class = 19;
const PDF: Class = 20;
const LRI: Class = 21;
const RLI: Class = 22;
const FSI: Class = 23;
const PDI: Class = 24;
const UNKNOWN_CLASS: Class = 255;

const BP_NONE: u8 = 0;
const BP_OPEN: u8 = 1;
const BP_CLOSE: u8 = 2;

const IMPLICIT_LEVEL: Level = -1;
const MAX_DEPTH: Level = 125;
const MAX_PAIRING_DEPTH: usize = 63;

/// `LookupRune`: (Class, bracket kind).
fn lookup(r: char) -> (Class, u8) {
    let value = u32::from(r);
    match BIDI.binary_search_by(|&(first, last, _, _)| {
        if value < first {
            std::cmp::Ordering::Greater
        } else if value > last {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    }) {
        Ok(index) => (BIDI[index].2, BIDI[index].3),
        // x/text's trie yields L (zero) for runes outside its table.
        Err(_) => (L, 0),
    }
}

fn is_in(c: Class, set: &[Class]) -> bool {
    set.contains(&c)
}

fn type_for_level(level: Level) -> Class {
    if level & 1 == 0 { L } else { R }
}

fn is_whitespace(c: Class) -> bool {
    matches!(
        c,
        LRE | RLE | LRO | RLO | PDF | LRI | RLI | FSI | PDI | BN | WS
    )
}

fn is_removed_by_x9(c: Class) -> bool {
    matches!(c, LRE | RLE | LRO | RLO | PDF | BN)
}

struct Paragraph {
    initial_types: Vec<Class>,
    pair_types: Vec<u8>,
    pair_values: Vec<u32>,
    embedding_level: Level,
    result_types: Vec<Class>,
    result_levels: Vec<Level>,
    matching_pdi: Vec<usize>,
    matching_isolate_initiator: Vec<isize>,
}

struct Sequence {
    indexes: Vec<usize>,
    types: Vec<Class>,
    resolved_levels: Vec<Level>,
    level: Level,
    sos: Class,
    eos: Class,
}

impl Paragraph {
    fn len(&self) -> usize {
        self.initial_types.len()
    }

    fn run(&mut self) {
        self.determine_matching_isolates();
        if self.embedding_level == IMPLICIT_LEVEL {
            self.embedding_level = self.determine_paragraph_embedding_level(0, self.len());
        }
        self.result_levels = vec![self.embedding_level; self.len()];
        self.determine_explicit_embedding_levels();
        let sequences = self.determine_isolating_run_sequences();
        for mut sequence in sequences {
            sequence.resolve_weak_types();
            self.resolve_paired_brackets(&mut sequence);
            sequence.resolve_neutral_types();
            sequence.resolve_implicit_levels();
            for (i, &x) in sequence.indexes.iter().enumerate() {
                self.result_types[x] = sequence.types[i];
                self.result_levels[x] = sequence.resolved_levels[i];
            }
        }
        self.assign_levels_to_characters_removed_by_x9();
    }

    fn determine_matching_isolates(&mut self) {
        let n = self.len();
        self.matching_pdi = vec![usize::MAX; n];
        self.matching_isolate_initiator = vec![-1; n];
        for i in 0..n {
            if is_in(self.result_types[i], &[LRI, RLI, FSI]) {
                let mut depth = 1;
                for j in i + 1..n {
                    let u = self.result_types[j];
                    if is_in(u, &[LRI, RLI, FSI]) {
                        depth += 1;
                    } else if u == PDI {
                        depth -= 1;
                        if depth == 0 {
                            self.matching_pdi[i] = j;
                            self.matching_isolate_initiator[j] = i as isize;
                            break;
                        }
                    }
                }
                if self.matching_pdi[i] == usize::MAX {
                    self.matching_pdi[i] = n;
                }
            }
        }
    }

    fn determine_paragraph_embedding_level(&self, start: usize, end: usize) -> Level {
        let mut strong = UNKNOWN_CLASS;
        let mut i = start;
        while i < end {
            let t = self.result_types[i];
            if is_in(t, &[L, AL, R]) {
                strong = t;
                break;
            } else if is_in(t, &[FSI, LRI, RLI]) {
                i = self.matching_pdi[i];
            }
            i += 1;
        }
        match strong {
            UNKNOWN_CLASS | L => 0,
            _ => 1,
        }
    }

    fn determine_explicit_embedding_levels(&mut self) {
        // directionalStatusStack
        let mut levels: Vec<Level> = Vec::with_capacity(MAX_DEPTH as usize + 1);
        let mut overrides: Vec<Class> = Vec::with_capacity(MAX_DEPTH as usize + 1);
        let mut isolates: Vec<bool> = Vec::with_capacity(MAX_DEPTH as usize + 1);
        let (mut overflow_isolates, mut overflow_embeddings, mut valid_isolates) = (0, 0, 0);
        levels.push(self.embedding_level);
        overrides.push(ON);
        isolates.push(false);
        for i in 0..self.len() {
            let t = self.result_types[i];
            match t {
                RLE | LRE | RLO | LRO | RLI | LRI | FSI => {
                    let is_isolate = is_in(t, &[RLI, LRI, FSI]);
                    let mut is_rtl = is_in(t, &[RLE, RLO, RLI]);
                    if t == FSI {
                        is_rtl = self
                            .determine_paragraph_embedding_level(i + 1, self.matching_pdi[i])
                            == 1;
                    }
                    if is_isolate {
                        self.result_levels[i] = *levels.last().unwrap();
                        if *overrides.last().unwrap() != ON {
                            self.result_types[i] = *overrides.last().unwrap();
                        }
                    }
                    let last = *levels.last().unwrap();
                    let new_level = if is_rtl {
                        (last + 1) | 1
                    } else {
                        (last + 2) & !1
                    };
                    if new_level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0
                    {
                        if is_isolate {
                            valid_isolates += 1;
                        }
                        levels.push(new_level);
                        overrides.push(match t {
                            LRO => L,
                            RLO => R,
                            _ => ON,
                        });
                        isolates.push(is_isolate);
                        if !is_isolate {
                            self.result_levels[i] = new_level;
                        }
                    } else if is_isolate {
                        overflow_isolates += 1;
                    } else if overflow_isolates == 0 {
                        overflow_embeddings += 1;
                    }
                }
                PDI => {
                    if overflow_isolates > 0 {
                        overflow_isolates -= 1;
                    } else if valid_isolates == 0 {
                    } else {
                        overflow_embeddings = 0;
                        while !*isolates.last().unwrap() {
                            levels.pop();
                            overrides.pop();
                            isolates.pop();
                        }
                        levels.pop();
                        overrides.pop();
                        isolates.pop();
                        valid_isolates -= 1;
                    }
                    self.result_levels[i] = *levels.last().unwrap();
                }
                PDF => {
                    self.result_levels[i] = *levels.last().unwrap();
                    if overflow_isolates > 0 {
                    } else if overflow_embeddings > 0 {
                        overflow_embeddings -= 1;
                    } else if !*isolates.last().unwrap() && levels.len() >= 2 {
                        levels.pop();
                        overrides.pop();
                        isolates.pop();
                    }
                }
                B => {
                    levels.clear();
                    overrides.clear();
                    isolates.clear();
                    overflow_isolates = 0;
                    overflow_embeddings = 0;
                    valid_isolates = 0;
                    self.result_levels[i] = self.embedding_level;
                }
                _ => {
                    self.result_levels[i] = *levels.last().unwrap();
                    if *overrides.last().unwrap() != ON {
                        self.result_types[i] = *overrides.last().unwrap();
                    }
                }
            }
        }
    }

    fn isolating_run_sequence(&self, indexes: Vec<usize>) -> Sequence {
        let types: Vec<Class> = indexes.iter().map(|&x| self.result_types[x]).collect();
        let mut prev_char = indexes[0] as isize - 1;
        while prev_char >= 0 && is_removed_by_x9(self.initial_types[prev_char as usize]) {
            prev_char -= 1;
        }
        let prev_level = if prev_char >= 0 {
            self.result_levels[prev_char as usize]
        } else {
            self.embedding_level
        };
        let last_type = *types.last().unwrap();
        let succ_level = if is_in(last_type, &[LRI, RLI, FSI]) {
            self.embedding_level
        } else {
            let mut limit = indexes.last().unwrap() + 1;
            while limit < self.len() && is_removed_by_x9(self.initial_types[limit]) {
                limit += 1;
            }
            if limit < self.len() {
                self.result_levels[limit]
            } else {
                self.embedding_level
            }
        };
        let level = self.result_levels[indexes[0]];
        Sequence {
            indexes,
            types,
            resolved_levels: Vec::new(),
            level,
            sos: type_for_level(prev_level.max(level)),
            eos: type_for_level(succ_level.max(level)),
        }
    }

    fn determine_level_runs(&self) -> Vec<Vec<usize>> {
        let mut run = Vec::new();
        let mut all = Vec::new();
        let mut current = IMPLICIT_LEVEL;
        for i in 0..self.len() {
            if !is_removed_by_x9(self.initial_types[i]) {
                if self.result_levels[i] != current {
                    if current >= 0 {
                        all.push(std::mem::take(&mut run));
                    }
                    current = self.result_levels[i];
                }
                run.push(i);
            }
        }
        if !run.is_empty() {
            all.push(run);
        }
        all
    }

    fn determine_isolating_run_sequences(&self) -> Vec<Sequence> {
        let level_runs = self.determine_level_runs();
        let mut run_for_character = vec![0usize; self.len()];
        for (i, run) in level_runs.iter().enumerate() {
            for &index in run {
                run_for_character[index] = i;
            }
        }
        let mut sequences = Vec::new();
        for run in &level_runs {
            let first = run[0];
            if self.initial_types[first] != PDI || self.matching_isolate_initiator[first] == -1 {
                let mut current: Vec<usize> = Vec::new();
                let mut run = run;
                loop {
                    current.extend_from_slice(run);
                    let last = *current.last().unwrap();
                    let last_type = self.initial_types[last];
                    if is_in(last_type, &[LRI, RLI, FSI]) && self.matching_pdi[last] != self.len() {
                        run = &level_runs[run_for_character[self.matching_pdi[last]]];
                    } else {
                        break;
                    }
                }
                sequences.push(self.isolating_run_sequence(current));
            }
        }
        sequences
    }

    fn assign_levels_to_characters_removed_by_x9(&mut self) {
        for i in 0..self.len() {
            let t = self.initial_types[i];
            if is_in(t, &[LRE, RLE, LRO, RLO, PDF, BN]) {
                self.result_types[i] = t;
                self.result_levels[i] = -1;
            }
        }
        if self.result_levels[0] == -1 {
            self.result_levels[0] = self.embedding_level;
        }
        for i in 1..self.len() {
            if self.result_levels[i] == -1 {
                self.result_levels[i] = self.result_levels[i - 1];
            }
        }
    }

    fn get_levels(&self, linebreaks: &[usize]) -> Vec<Level> {
        let mut result = self.result_levels.clone();
        for i in 0..self.len() {
            if is_in(self.initial_types[i], &[B, S]) {
                result[i] = self.embedding_level;
                for j in (0..i).rev() {
                    if is_whitespace(self.initial_types[j]) {
                        result[j] = self.embedding_level;
                    } else {
                        break;
                    }
                }
            }
        }
        let mut start = 0;
        for &limit in linebreaks {
            for j in (start..limit).rev() {
                if is_whitespace(self.initial_types[j]) {
                    result[j] = self.embedding_level;
                } else {
                    break;
                }
            }
            start = limit;
        }
        result
    }

    /// `resolvePairedBrackets` (bracket.go).
    fn resolve_paired_brackets(&self, s: &mut Sequence) {
        let dir_embed = if s.level & 1 != 0 { R } else { L };
        // locateBrackets
        let mut openers: Vec<usize> = Vec::new(); // front = last element
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for (i, &index) in s.indexes.iter().enumerate() {
            if self.pair_types[index] == BP_NONE || s.types[i] != ON {
                continue;
            }
            match self.pair_types[index] {
                BP_OPEN => {
                    if openers.len() == MAX_PAIRING_DEPTH {
                        openers.clear();
                        // Go returns from locateBrackets: pairs found so far
                        // are still resolved.
                        break;
                    }
                    openers.push(i);
                }
                BP_CLOSE => {
                    let mut count = 0;
                    for position in (0..openers.len()).rev() {
                        count += 1;
                        let opener = openers[position];
                        if self.pair_values[s.indexes[opener]] == self.pair_values[s.indexes[i]] {
                            pairs.push((opener, i));
                            openers.truncate(openers.len() - count);
                            break;
                        }
                    }
                    pairs.sort_by_key(|&(opener, _)| opener);
                }
                _ => {}
            }
        }
        // resolveBrackets
        for (opener, closer) in pairs {
            let strong = |types: &[Class], index: usize| match types[index] {
                EN | AN | AL | R => R,
                L => L,
                _ => ON,
            };
            // classifyPairContent
            let mut dir_pair = ON;
            for i in opener + 1..closer {
                let dir = strong(&s.types, i);
                if dir == ON {
                    continue;
                }
                if dir == dir_embed {
                    dir_pair = dir;
                    break;
                }
                dir_pair = dir;
            }
            if dir_pair == ON {
                continue;
            }
            if dir_pair != dir_embed {
                // classBeforePair
                dir_pair = s.sos;
                for i in (0..opener).rev() {
                    let dir = strong(&s.types, i);
                    if dir != ON {
                        dir_pair = dir;
                        break;
                    }
                }
                if dir_pair == dir_embed || dir_pair == ON {
                    dir_pair = dir_embed;
                }
            }
            // setBracketsToType
            s.types[opener] = dir_pair;
            s.types[closer] = dir_pair;
            for i in opener + 1..closer {
                if self.initial_types[s.indexes[i]] != NSM {
                    break;
                }
                s.types[i] = dir_pair;
            }
            for i in closer + 1..s.indexes.len() {
                if self.initial_types[s.indexes[i]] != NSM {
                    break;
                }
                s.types[i] = dir_pair;
            }
        }
    }
}

impl Sequence {
    fn find_run_limit(&self, mut index: usize, valid: &[Class]) -> usize {
        while index < self.types.len() {
            if !valid.contains(&self.types[index]) {
                return index;
            }
            index += 1;
        }
        self.types.len()
    }

    fn resolve_weak_types(&mut self) {
        let n = self.types.len();
        // W1
        let mut preceding = self.sos;
        for i in 0..n {
            let t = self.types[i];
            if t == NSM {
                self.types[i] = preceding;
            } else {
                preceding = t;
            }
        }
        // W2
        for i in 0..n {
            if self.types[i] == EN {
                for j in (0..i).rev() {
                    let t = self.types[j];
                    if is_in(t, &[L, R, AL]) {
                        if t == AL {
                            self.types[i] = AN;
                        }
                        break;
                    }
                }
            }
        }
        // W3
        for t in &mut self.types {
            if *t == AL {
                *t = R;
            }
        }
        // W4
        for i in 1..n.saturating_sub(1) {
            let t = self.types[i];
            if t == ES || t == CS {
                let prev = self.types[i - 1];
                let succ = self.types[i + 1];
                if prev == EN && succ == EN {
                    self.types[i] = EN;
                } else if self.types[i] == CS && prev == AN && succ == AN {
                    self.types[i] = AN;
                }
            }
        }
        // W5 (Go's `i = runEnd` inside a range loop does not skip)
        for i in 0..n {
            if self.types[i] == ET {
                let run_start = i;
                let run_end = self.find_run_limit(run_start, &[ET]);
                let mut t = if run_start > 0 {
                    self.types[run_start - 1]
                } else {
                    self.sos
                };
                if t != EN {
                    t = if run_end < n {
                        self.types[run_end]
                    } else {
                        self.eos
                    };
                }
                if t == EN {
                    for value in &mut self.types[run_start..run_end] {
                        *value = EN;
                    }
                }
            }
        }
        // W6
        for t in &mut self.types {
            if is_in(*t, &[ES, ET, CS]) {
                *t = ON;
            }
        }
        // W7
        for i in 0..n {
            if self.types[i] == EN {
                let mut prev_strong = self.sos;
                for j in (0..i).rev() {
                    let t = self.types[j];
                    if t == L || t == R {
                        prev_strong = t;
                        break;
                    }
                }
                if prev_strong == L {
                    self.types[i] = L;
                }
            }
        }
    }

    fn resolve_neutral_types(&mut self) {
        let n = self.types.len();
        const NEUTRALS: [Class; 8] = [B, S, WS, ON, RLI, LRI, FSI, PDI];
        for i in 0..n {
            if NEUTRALS.contains(&self.types[i]) {
                let run_start = i;
                let run_end = self.find_run_limit(run_start, &NEUTRALS);
                let lead = if run_start == 0 {
                    self.sos
                } else {
                    let t = self.types[run_start - 1];
                    if t == AN || t == EN { R } else { t }
                };
                let trail = if run_end == n {
                    self.eos
                } else {
                    let t = self.types[run_end];
                    if t == AN || t == EN { R } else { t }
                };
                let resolved = if lead == trail {
                    lead
                } else {
                    type_for_level(self.level)
                };
                for value in &mut self.types[run_start..run_end] {
                    *value = resolved;
                }
            }
        }
    }

    fn resolve_implicit_levels(&mut self) {
        self.resolved_levels = vec![self.level; self.types.len()];
        if self.level & 1 == 0 {
            for (i, &t) in self.types.iter().enumerate() {
                if t == L {
                } else if t == R {
                    self.resolved_levels[i] += 1;
                } else {
                    self.resolved_levels[i] += 2;
                }
            }
        } else {
            for (i, &t) in self.types.iter().enumerate() {
                if t != R {
                    self.resolved_levels[i] += 1;
                }
            }
        }
    }
}

/// `Paragraph.SetString(text, DefaultDirection(LeftToRight))` + `Order()`:
/// runs as (start rune, end rune exclusive, right-to-left), or None when
/// x/text reports no runs.
pub(crate) fn order(runes: &[char]) -> Option<Vec<(usize, usize, bool)>> {
    let mut types = Vec::with_capacity(runes.len());
    let mut pair_types = Vec::with_capacity(runes.len());
    let mut pair_values = Vec::with_capacity(runes.len());
    for &r in runes {
        let (class, bracket) = lookup(r);
        if class == B {
            break;
        }
        types.push(class);
        match bracket {
            1 => {
                pair_types.push(BP_OPEN);
                pair_values.push(u32::from(r));
            }
            2 => {
                pair_types.push(BP_CLOSE);
                pair_values.push(u32::from(r));
            }
            _ => {
                pair_types.push(BP_NONE);
                pair_values.push(0);
            }
        }
    }
    if types.is_empty() {
        return None;
    }
    // DefaultDirection(LeftToRight) leaves the paragraph level implicit.
    let mut paragraph = Paragraph {
        result_types: types.clone(),
        initial_types: types,
        pair_types,
        pair_values,
        embedding_level: IMPLICIT_LEVEL,
        result_levels: Vec::new(),
        matching_pdi: Vec::new(),
        matching_isolate_initiator: Vec::new(),
    };
    paragraph.run();
    let levels = paragraph.get_levels(&[paragraph.len()]);
    // calculateOrdering: split on direction (level parity) changes; the
    // final run extends over every remaining rune.
    let mut runs = Vec::new();
    let mut prev_rtl: Option<bool> = None;
    let mut prev_i = 0;
    for (i, &level) in levels.iter().enumerate() {
        let rtl = level % 2 != 0;
        if prev_rtl != Some(rtl) {
            if i > 0 {
                runs.push((prev_i, i, prev_rtl.unwrap()));
            }
            prev_i = i;
            prev_rtl = Some(rtl);
        }
    }
    runs.push((prev_i, runes.len(), prev_rtl.unwrap_or(false)));
    Some(runs)
}

#[cfg(test)]
mod tests {
    use super::order;

    fn runs(text: &str) -> Vec<(usize, usize, bool)> {
        order(&text.chars().collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn runs_match_x_text_ordering() {
        // Observed from golang.org/x/text v0.41.0 Paragraph.Order: x/text
        // never pairs "(" with ")" (matchOpener compares the runes
        // themselves), so N0 does not make the closing bracket RTL.
        assert_eq!(
            runs("Hello \u{5e9}\u{5dc}\u{5d5}\u{5dd} (123) world"),
            [(0, 6, false), (6, 12, true), (12, 22, false)]
        );
        assert_eq!(
            runs("abc \u{5d0}\u{5d1}\u{5d2} \u{661}\u{662}\u{663} def"),
            [(0, 4, false), (4, 8, true), (8, 15, false)]
        );
        // The paragraph level is implicit: leading RTL text makes level 1.
        assert_eq!(runs("\u{5d0} a"), [(0, 2, true), (2, 3, false)]);
        // A paragraph separator ends the classified text; the last run
        // still covers every remaining rune.
        assert_eq!(runs("a\u{2029}\u{5d0}"), [(0, 3, false)]);
        assert!(order(&['\u{2029}']).is_none());
    }
}

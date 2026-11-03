//! `hex history --search`: stemmed, fuzzy BM25 over dictation texts, with
//! quoted phrases and the vocabulary's likely mis-transcriptions.

use std::collections::{HashMap, HashSet};

use color_eyre::eyre::{Result, eyre};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::dictation::vocabulary::VocabularyEntry;

const SNIPPET_TOKENS: usize = 36;
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

struct Passage {
    text: String,
    tokens: Vec<Token>,
    frequencies: HashMap<String, u32>,
}

struct Token {
    stem: String,
    start: usize,
    end: usize,
}

impl Passage {
    fn new(text: String) -> Self {
        let tokens = tokenize(&text);
        let mut frequencies = HashMap::new();
        for token in &tokens {
            *frequencies.entry(token.stem.clone()).or_insert(0) += 1;
        }
        Self {
            text,
            tokens,
            frequencies,
        }
    }
}

/// Lowercase without diacritics: `Café` -> `cafe`.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for character in text.chars() {
        for lower in character.to_lowercase() {
            folded.extend(
                lower
                    .nfkd()
                    .filter(|character| !is_combining_mark(*character)),
            );
        }
    }
    folded
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut word = String::new();
    let mut finish = |start: &mut Option<usize>, word: &mut String, end: usize| {
        if let Some(begin) = start.take() {
            let folded = fold(word);
            if !folded.is_empty() {
                tokens.push(Token {
                    stem: stem(&folded),
                    start: begin,
                    end,
                });
            }
            word.clear();
        }
    };
    let mut last_end = 0;
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            start.get_or_insert(index);
            word.push(character);
            last_end = index + character.len_utf8();
        } else if matches!(character, '\'' | '’') && start.is_some() {
            // `don't` and `Dax's` stay one word.
        } else {
            finish(&mut start, &mut word, last_end);
        }
    }
    finish(&mut start, &mut word, last_end);
    tokens
}

/// A light English stemmer: `scheduled`, `schedules`, and `scheduling` all
/// become `schedul`; `recurring` and `recur` become `recur`.
fn stem(word: &str) -> String {
    let mut stem = word.to_string();
    if stem.chars().count() <= 3 || stem.bytes().any(|byte| byte.is_ascii_digit()) {
        return stem;
    }
    for (suffix, minimum, replacement) in [
        ("ingly", 3, ""),
        ("edly", 3, ""),
        ("ings", 3, ""),
        ("ing", 3, ""),
        ("ied", 2, "y"),
        ("ies", 2, "y"),
        ("ed", 3, ""),
        ("es", 3, ""),
        ("ly", 3, ""),
        ("s", 3, ""),
    ] {
        if stem.ends_with(suffix)
            && stem.len() - suffix.len() >= minimum
            && !(suffix == "s" && (stem.ends_with("ss") || stem.ends_with("us")))
        {
            stem.truncate(stem.len() - suffix.len());
            stem.push_str(replacement);
            break;
        }
    }
    let bytes = stem.as_bytes();
    if bytes.len() >= 4 {
        let (last, previous) = (bytes[bytes.len() - 1], bytes[bytes.len() - 2]);
        if last == previous && last.is_ascii_lowercase() && !b"aeioulsz".contains(&last) {
            stem.pop();
        }
    }
    if stem.len() >= 4 && stem.ends_with('e') {
        stem.pop();
    }
    stem
}

const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "did", "do", "for", "from",
    "had", "has", "have", "he", "i", "in", "is", "it", "its", "of", "on", "or", "our", "she", "so",
    "that", "the", "their", "them", "there", "they", "this", "to", "was", "we", "were", "what",
    "when", "where", "which", "who", "with", "you",
];

fn soundex(word: &str) -> Option<String> {
    let mut key = sound_key(word)?;
    key.truncate(4);
    while key.len() < 4 {
        key.push('0');
    }
    Some(key)
}

/// Soundex without its four-character cap or padding: the first letter, then
/// every consonant sound (`cappuccino` and `capuchin` are both `c125`).
fn sound_key(word: &str) -> Option<String> {
    let mut characters = word.chars().filter(char::is_ascii_alphabetic);
    let first = characters.next()?;
    let code = |character: char| match character {
        'b' | 'f' | 'p' | 'v' => Some('1'),
        'c' | 'g' | 'j' | 'k' | 'q' | 's' | 'x' | 'z' => Some('2'),
        'd' | 't' => Some('3'),
        'l' => Some('4'),
        'm' | 'n' => Some('5'),
        'r' => Some('6'),
        _ => None,
    };
    let mut key = String::from(first);
    let mut previous = code(first);
    for character in characters {
        let current = code(character);
        if let Some(digit) = current
            && current != previous
        {
            key.push(digit);
        }
        if !matches!(character, 'h' | 'w') {
            previous = current;
        }
    }
    Some(key)
}

/// Optimal string alignment distance, or `None` when above `limit`.
fn edit_distance(left: &str, right: &str, limit: usize) -> Option<usize> {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.len().abs_diff(right.len()) > limit {
        return None;
    }
    let width = right.len() + 1;
    let mut rows = vec![vec![0; width]; left.len() + 1];
    for (index, row) in rows.iter_mut().enumerate() {
        row[0] = index;
    }
    for (index, cell) in rows[0].iter_mut().enumerate() {
        *cell = index;
    }
    for i in 1..=left.len() {
        for j in 1..=right.len() {
            let cost = usize::from(left[i - 1] != right[j - 1]);
            let mut value = (rows[i - 1][j] + 1)
                .min(rows[i][j - 1] + 1)
                .min(rows[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && left[i - 1] == right[j - 2] && left[i - 2] == right[j - 1] {
                value = value.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = value;
        }
    }
    let distance = rows[left.len()][right.len()];
    (distance <= limit).then_some(distance)
}

#[derive(Debug, Default)]
struct ParsedQuery {
    terms: Vec<String>,
    phrases: Vec<Vec<String>>,
}

impl ParsedQuery {
    /// Unquoted words are ranked terms; `"quoted words"` are required exact
    /// phrases (stem-insensitive, never fuzzy).
    fn parse(query: &str) -> Self {
        let mut parsed = Self::default();
        let mut words = Vec::new();
        for (index, part) in query.split('"').enumerate() {
            let stems = tokenize(part)
                .into_iter()
                .map(|token| token.stem)
                .collect::<Vec<_>>();
            if index % 2 == 1 {
                if !stems.is_empty() {
                    parsed.phrases.push(stems);
                }
            } else {
                words.extend(stems);
            }
        }
        let content = words
            .iter()
            .filter(|word| !STOPWORDS.contains(&word.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        parsed.terms = if content.is_empty() && parsed.phrases.is_empty() {
            words
        } else {
            content
        };
        let mut seen = HashSet::new();
        parsed.terms.retain(|term| seen.insert(term.clone()));
        parsed
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.phrases.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MatchKind {
    Exact,
    Prefix,
    /// A spelling variant this many edits away.
    Edit(usize),
    Sound,
    /// A vocabulary term's "sounds like" spelling, or the term as two words.
    Alias,
    /// A word that sounds like a vocabulary term ([`sounds_alike`]).
    Phonetic,
}

impl MatchKind {
    fn weight(self) -> f32 {
        match self {
            Self::Exact => 1.0,
            Self::Prefix | Self::Alias => 0.5,
            Self::Edit(_) => 0.35,
            Self::Phonetic => 0.3,
            Self::Sound => 0.2,
        }
    }
}

/// Spelling fallbacks (edit distance and Soundex) apply only to a term found
/// exactly in fewer passages than this: with more, the transcripts already
/// spell the word that way and its near neighbours are other words.
const RARE_EXACT_PASSAGES: u32 = 3;
/// A spelling fallback is skipped when it is a common word: in more than this
/// share of the passages searched, and at least `COMMON_MIN_PASSAGES`.
const COMMON_SHARE: f32 = 0.005;
const COMMON_MIN_PASSAGES: f32 = 5.0;

/// Whether an expansion found by [`expand`] is used. Prefixes are kept like
/// stems. Spelling fallbacks need a term whose exact hits are rare and a
/// candidate that is not a common word, except that a term with no exact hit
/// at all still reaches a common word one edit away (a plain typo). A
/// vocabulary alias the person wrote is always used; a phonetic one only when
/// it is not a common word, however often the term itself appears.
fn admits(kind: MatchKind, exact_passages: u32, passages: u32, common: f32) -> bool {
    match kind {
        MatchKind::Exact | MatchKind::Prefix | MatchKind::Alias => true,
        MatchKind::Phonetic => passages as f32 <= common,
        MatchKind::Edit(_) | MatchKind::Sound => {
            exact_passages < RARE_EXACT_PASSAGES
                && (passages as f32 <= common
                    || (kind == MatchKind::Edit(1) && exact_passages == 0))
        }
    }
}

fn expand<'a>(
    term: &str,
    vocabulary: impl Iterator<Item = &'a String>,
) -> Vec<(String, MatchKind)> {
    let term_sound = soundex(term);
    let length = term.chars().count();
    let mut expansions = Vec::new();
    for candidate in vocabulary {
        let kind = if candidate == term {
            Some(MatchKind::Exact)
        } else {
            let candidate_length = candidate.chars().count();
            if (length >= 4 && candidate.starts_with(term) && candidate_length <= length + 5)
                || (candidate_length >= 5
                    && term.starts_with(candidate.as_str())
                    && length - candidate_length <= 3)
            {
                Some(MatchKind::Prefix)
            } else if let Some(distance) = (length >= 4)
                .then(|| edit_distance(term, candidate, if length >= 8 { 2 } else { 1 }))
                .flatten()
            {
                Some(MatchKind::Edit(distance))
            } else if (3..=5).contains(&length)
                && (3..=6).contains(&candidate_length)
                && length.abs_diff(candidate_length) <= 2
                && term_sound.is_some()
                && soundex(candidate) == term_sound
            {
                Some(MatchKind::Sound)
            } else {
                None
            }
        };
        if let Some(kind) = kind {
            expansions.push((candidate.clone(), kind));
        }
    }
    expansions.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    expansions
}

/// Transcript stems, one word or an adjacent phrase, that stand for a
/// vocabulary term.
struct Alias {
    stems: Vec<String>,
    kind: MatchKind,
}

/// The aliases a query `term` gets when it names a one-word vocabulary term:
/// the term's "sounds like" spellings, the term split into two adjacent words
/// found in `corpus` (`open code` for OpenCode), and words in `corpus` that
/// sound like it ([`sounds_alike`]) and pass [`admits`]. Multi-word terms get
/// none; their words are searched like any other.
fn vocabulary_aliases(
    term: &str,
    vocabulary: &[VocabularyEntry],
    corpus: &HashMap<&String, u32>,
    common: f32,
) -> Vec<Alias> {
    let mut aliases: Vec<Alias> = Vec::new();
    let mut add = |stems: Vec<String>, kind: MatchKind| {
        if stems.is_empty() || stems == [term] {
            return;
        }
        match aliases.iter_mut().find(|alias| alias.stems == stems) {
            Some(alias) if alias.kind.weight() < kind.weight() => alias.kind = kind,
            Some(_) => {}
            None => aliases.push(Alias { stems, kind }),
        }
    };
    for entry in vocabulary {
        let tokens = tokenize(&entry.term);
        let [token] = tokens.as_slice() else {
            continue;
        };
        if token.stem != term {
            continue;
        }
        for spelling in &entry.sounds_like {
            add(
                tokenize(spelling)
                    .into_iter()
                    .map(|token| token.stem)
                    .collect(),
                MatchKind::Alias,
            );
        }
        let word = fold(&entry.term[token.start..token.end])
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect::<String>();
        let boundaries = word
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for &split in boundaries.iter().skip(3) {
            if word[split..].chars().count() < 3 {
                break;
            }
            let (left, right) = (stem(&word[..split]), stem(&word[split..]));
            if corpus.contains_key(&left) && corpus.contains_key(&right) {
                add(vec![left, right], MatchKind::Alias);
            }
        }
        for (candidate, passages) in corpus {
            if sounds_alike(term, candidate) && admits(MatchKind::Phonetic, 0, *passages, common) {
                add(vec![(*candidate).clone()], MatchKind::Phonetic);
            }
        }
    }
    aliases.sort_unstable_by(|left, right| left.stems.cmp(&right.stems));
    aliases
}

/// Consonant sounds a vocabulary term needs before words that merely sound
/// like it count as hits. Names with fewer (Rune, Dax) match too many words
/// this way and rely on "sounds like" spellings.
const SOUND_ALIKE_MIN_CONSONANTS: usize = 2;

/// Whether the corpus stem `candidate` is a likely mis-transcription of the
/// vocabulary stem `term` (`capuchin` for `cappuccino`): the same uncapped
/// Soundex key with at least [`SOUND_ALIKE_MIN_CONSONANTS`] consonant sounds,
/// at most half the letters changed, and not a stopword.
fn sounds_alike(term: &str, candidate: &str) -> bool {
    let length = term.chars().count().max(candidate.chars().count());
    candidate != term
        && candidate.chars().count() >= 4
        && !STOPWORDS.contains(&candidate)
        // The key is the first letter, then one digit per consonant sound.
        && sound_key(term).is_some_and(|key| {
            key.len() > SOUND_ALIKE_MIN_CONSONANTS && sound_key(candidate) == Some(key)
        })
        && edit_distance(term, candidate, length / 2).is_some()
}

struct Scored {
    passage: usize,
    score: f32,
    /// Every query term matched an exact stem.
    exact: bool,
    matched: Vec<usize>,
}

/// Best first: a passage matching every term exactly outranks one that needs
/// a fuzzy match, whatever their scores; then by score.
fn relevance(left: &Scored, right: &Scored) -> std::cmp::Ordering {
    right
        .exact
        .cmp(&left.exact)
        .then_with(|| right.score.total_cmp(&left.score))
}

/// Which of two matches in a passage stands for a query term: the exact stem
/// whenever present, then the higher score, the more trusted kind, and the
/// alphabetically first stem, so equal matches resolve the same way every run.
fn stands_for_term(
    (score, stem, kind): (f32, &str, MatchKind),
    (other_score, other_stem, other_kind): (f32, &str, MatchKind),
) -> std::cmp::Ordering {
    (kind == MatchKind::Exact)
        .cmp(&(other_kind == MatchKind::Exact))
        .then(score.total_cmp(&other_score))
        .then(kind.weight().total_cmp(&other_kind.weight()))
        .then(other_stem.cmp(stem))
}

/// BM25 over `passages` as one corpus, with stemmed, fuzzy terms.
/// `owners[i]` is passage `i`'s document: alternative texts of one document
/// (contiguous, such as a raw and a corrected transcript) count once in
/// document frequencies. `vocabulary` aliases never count as exact.
/// Unsorted; passages that match nothing are omitted.
fn score(
    passages: &[&Passage],
    owners: &[usize],
    query: &ParsedQuery,
    vocabulary: &[VocabularyEntry],
) -> Vec<Scored> {
    debug_assert_eq!(passages.len(), owners.len());
    // Per stem: documents containing it, and the last document counted.
    let mut counted: HashMap<&String, (u32, usize)> = HashMap::new();
    let mut total_length = 0_usize;
    let mut documents = 0_u32;
    for (index, (passage, owner)) in passages.iter().zip(owners).enumerate() {
        total_length += passage.tokens.len();
        if index == 0 || owners[index - 1] != *owner {
            documents += 1;
        }
        for stem in passage.frequencies.keys() {
            let (count, last) = counted.entry(stem).or_insert((0, usize::MAX));
            if *last != *owner {
                *count += 1;
                *last = *owner;
            }
        }
    }
    if passages.is_empty() {
        return Vec::new();
    }
    let document_frequency = counted
        .into_iter()
        .map(|(stem, (count, _))| (stem, count))
        .collect::<HashMap<_, _>>();
    let average_length = total_length as f32 / passages.len() as f32;
    let idf = |stem: &str| {
        let frequency = document_frequency
            .get(&stem.to_string())
            .copied()
            .unwrap_or(0) as f32;
        (1.0 + (documents as f32 - frequency + 0.5) / (frequency + 0.5)).ln()
    };
    let passages_with = |stem: &String| document_frequency.get(stem).copied().unwrap_or(0);
    let common = (documents as f32 * COMMON_SHARE).max(COMMON_MIN_PASSAGES);
    let expansions = query
        .terms
        .iter()
        .map(|term| {
            let exact_passages = passages_with(term);
            let mut stems = expand(term, document_frequency.keys().copied())
                .into_iter()
                .filter(|(stem, kind)| admits(*kind, exact_passages, passages_with(stem), common))
                .collect::<Vec<_>>();
            let mut phrases = Vec::new();
            for alias in vocabulary_aliases(term, vocabulary, &document_frequency, common) {
                let [stem] = alias.stems.as_slice() else {
                    phrases.push(alias);
                    continue;
                };
                stems.retain(|(other, kind)| other != stem || kind.weight() >= alias.kind.weight());
                if !stems.iter().any(|(other, _)| other == stem) {
                    stems.push((stem.clone(), alias.kind));
                }
            }
            let stems = stems
                .into_iter()
                .map(|(stem, kind)| {
                    let weight = kind.weight() * idf(&stem);
                    (stem, kind, weight)
                })
                .collect::<Vec<_>>();
            let phrases = phrases
                .into_iter()
                .map(|alias| {
                    let weight =
                        alias.kind.weight() * alias.stems.iter().map(|stem| idf(stem)).sum::<f32>();
                    let label = alias.stems.join(" ");
                    (alias, label, weight)
                })
                .collect::<Vec<_>>();
            (stems, phrases)
        })
        .collect::<Vec<_>>();
    let mut hits = Vec::new();
    for (index, passage) in passages.iter().enumerate() {
        let length_norm =
            1.0 - BM25_B + BM25_B * passage.tokens.len() as f32 / average_length.max(1.0);
        let saturate = |frequency: u32| {
            let frequency = frequency as f32;
            frequency * (BM25_K1 + 1.0) / (frequency + BM25_K1 * length_norm)
        };
        let mut matched_stems: HashSet<&str> = HashSet::new();
        let mut matched_positions = Vec::new();
        let mut phrase_score = 0.0;
        let mut missing_phrase = false;
        for phrase in &query.phrases {
            let starts = phrase_starts(passage, phrase);
            if starts.is_empty() {
                missing_phrase = true;
                break;
            }
            let phrase_idf = phrase.iter().map(|stem| idf(stem)).sum::<f32>();
            phrase_score += 1.5 * phrase_idf * saturate(starts.len() as u32);
            for start in starts {
                matched_positions.extend(start..start + phrase.len());
            }
        }
        if missing_phrase {
            continue;
        }
        let mut term_score = 0.0;
        let mut matched_weight = 0.0;
        let mut exact_terms = Vec::with_capacity(query.terms.len());
        for (candidates, phrases) in &expansions {
            let mut best: Option<(f32, &str, MatchKind)> = None;
            for (stem, kind, weight) in candidates {
                if let Some(frequency) = passage.frequencies.get(stem) {
                    matched_stems.insert(stem);
                    let candidate = (weight * saturate(*frequency), stem.as_str(), *kind);
                    if best.is_none_or(|best| stands_for_term(candidate, best).is_gt()) {
                        best = Some(candidate);
                    }
                }
            }
            for (alias, label, weight) in phrases {
                let starts = phrase_starts(passage, &alias.stems);
                if starts.is_empty() {
                    continue;
                }
                for start in &starts {
                    matched_positions.extend(*start..*start + alias.stems.len());
                }
                let candidate = (
                    weight * saturate(starts.len() as u32),
                    label.as_str(),
                    alias.kind,
                );
                if best.is_none_or(|best| stands_for_term(candidate, best).is_gt()) {
                    best = Some(candidate);
                }
            }
            exact_terms.push(matches!(best, Some((_, _, MatchKind::Exact))));
            if let Some((score, _, kind)) = best {
                term_score += score;
                matched_weight += kind.weight();
            }
        }
        if term_score == 0.0 && phrase_score == 0.0 {
            continue;
        }
        // Fuzzy matches cover a term only as far as they are trusted.
        let coverage = if query.terms.is_empty() {
            1.0
        } else {
            0.5 + 0.5 * matched_weight / query.terms.len() as f32
        };
        let mut adjacency = 0.0;
        for pair in 0..query.terms.len().saturating_sub(1) {
            if !(exact_terms[pair] && exact_terms[pair + 1]) {
                continue;
            }
            let (first, second) = (&query.terms[pair], &query.terms[pair + 1]);
            if passage
                .tokens
                .windows(2)
                .any(|window| window[0].stem == *first && window[1].stem == *second)
            {
                adjacency += 0.5 * (idf(first) + idf(second));
            }
        }
        for (position, token) in passage.tokens.iter().enumerate() {
            if matched_stems.contains(token.stem.as_str()) {
                matched_positions.push(position);
            }
        }
        matched_positions.sort_unstable();
        matched_positions.dedup();
        hits.push(Scored {
            passage: index,
            score: term_score * coverage + phrase_score + adjacency,
            exact: exact_terms.iter().all(|exact| *exact),
            matched: matched_positions,
        });
    }
    hits
}

/// One ranked document from [`search_texts`], through its best text.
#[derive(Debug)]
pub struct TextHit {
    /// The document's index.
    pub index: usize,
    pub score: f32,
    /// A window of the best text around the matches, which are wrapped in `**`.
    pub snippet: String,
}

/// Rank documents of plain text with stemmed, fuzzy BM25 and quoted-phrase
/// rules. A document's texts are alternatives, such as a raw and a corrected
/// transcript: they count as one document for term statistics, and the best
/// of them is its hit. Best first; ties keep input order.
pub fn search_texts(
    documents: &[Vec<&str>],
    query: &str,
    vocabulary: &[VocabularyEntry],
) -> Result<Vec<TextHit>> {
    let parsed = ParsedQuery::parse(query);
    if parsed.is_empty() {
        return Err(eyre!("The search query has no words."));
    }
    let mut owners = Vec::new();
    let mut passages = Vec::new();
    for (document, texts) in documents.iter().enumerate() {
        for text in texts {
            owners.push(document);
            passages.push(Passage::new((*text).to_owned()));
        }
    }
    let texts = passages.iter().collect::<Vec<_>>();
    let mut scored = score(&texts, &owners, &parsed, vocabulary);
    scored.sort_by(|left, right| relevance(left, right).then(left.passage.cmp(&right.passage)));
    let mut seen = HashSet::new();
    Ok(scored
        .into_iter()
        .filter(|scored| seen.insert(owners[scored.passage]))
        .map(|scored| TextHit {
            index: owners[scored.passage],
            score: scored.score,
            snippet: snippet(&passages[scored.passage], &scored.matched),
        })
        .collect())
}

fn phrase_starts(passage: &Passage, phrase: &[String]) -> Vec<usize> {
    if phrase.is_empty() || passage.tokens.len() < phrase.len() {
        return Vec::new();
    }
    (0..=passage.tokens.len() - phrase.len())
        .filter(|start| {
            phrase
                .iter()
                .enumerate()
                .all(|(offset, stem)| passage.tokens[start + offset].stem == *stem)
        })
        .collect()
}

fn snippet(passage: &Passage, matched: &[usize]) -> String {
    let tokens = &passage.tokens;
    if tokens.is_empty() {
        return passage.text.clone();
    }
    let window = SNIPPET_TOKENS.min(tokens.len());
    let best_start = (0..=tokens.len() - window)
        .max_by_key(|start| {
            let count = matched
                .iter()
                .filter(|&&position| (*start..*start + window).contains(&position))
                .count();
            // Prefer windows that lead into the first match a little.
            (
                count,
                std::cmp::Reverse(
                    matched
                        .first()
                        .map_or(0, |first| first.saturating_sub(4).abs_diff(*start)),
                ),
            )
        })
        .unwrap_or(0);
    let end = best_start + window;
    let text_start = if best_start == 0 {
        0
    } else {
        tokens[best_start].start
    };
    let text_end = if end == tokens.len() {
        passage.text.len()
    } else {
        tokens[end - 1].end
    };
    let mut output = String::new();
    if best_start > 0 {
        output.push('…');
    }
    let mut cursor = text_start;
    for (position, token) in tokens.iter().enumerate().take(end).skip(best_start) {
        if !matched.contains(&position) {
            continue;
        }
        output.push_str(&passage.text[cursor..token.start]);
        output.push_str("**");
        output.push_str(&passage.text[token.start..token.end]);
        output.push_str("**");
        cursor = token.end;
    }
    output.push_str(&passage.text[cursor..text_end]);
    if end < tokens.len() {
        output.push('…');
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_term_found_in_fewer_passages_still_reaches_its_neighbours() {
        // The fixture's `blob` story with only two exact passages.
        let texts = [
            "keep the images in blob storage",
            "the blob store sits behind a CDN",
            "I wrote a blog post",
            "the onboarding video is done",
        ];
        let documents = texts.map(|text| vec![text]);
        let matched = |documents: &[Vec<&str>]| {
            search_texts(documents, "blob", &[])
                .unwrap()
                .into_iter()
                .map(|hit| hit.index)
                .collect::<Vec<_>>()
        };
        assert_eq!(matched(&documents), [0, 1, 2]);
        let mut common = documents.to_vec();
        common.push(vec!["one more blob"]);
        assert_eq!(matched(&common), [4, 0, 1]);
    }

    #[test]
    fn an_all_exact_passage_outranks_a_higher_scoring_partial_one() {
        // `storage` is rare and `blob` common, so a short passage repeating
        // `storage` alone outscores a long one that has both words.
        let mut texts = vec![
            "blob first, then a long tangent about the quarter, the hiring plan, the offsite, the \
             budget review, and the roadmap before we finally got back to storage",
            "storage storage storage storage",
        ];
        texts.extend(["the blob again"; 8]);
        let documents = texts.iter().map(|text| vec![*text]).collect::<Vec<_>>();
        let hits = search_texts(&documents, "blob storage", &[]).unwrap();
        assert_eq!(hits[0].index, 0);
        let partial = hits.iter().find(|hit| hit.index == 1).unwrap();
        assert!(partial.score > hits[0].score, "{hits:?}");
    }

    #[test]
    fn spelling_fallbacks_need_rare_terms_and_uncommon_candidates() {
        let common = 5.0;
        assert!(admits(MatchKind::Edit(1), 2, 4, common));
        assert!(admits(MatchKind::Edit(2), 2, 4, common));
        // The term is spelled this way often enough; neighbours are other words.
        assert!(!admits(MatchKind::Edit(1), RARE_EXACT_PASSAGES, 1, common));
        assert!(!admits(MatchKind::Sound, RARE_EXACT_PASSAGES, 1, common));
        // `made` for `media`.
        assert!(!admits(MatchKind::Sound, 1, 9, common));
        assert!(!admits(MatchKind::Edit(1), 1, 9, common));
        // A plain typo of a common word still reaches it, but only by one edit.
        assert!(admits(MatchKind::Edit(1), 0, 9, common));
        assert!(!admits(MatchKind::Edit(2), 0, 9, common));
        assert!(!admits(MatchKind::Sound, 0, 9, common));
        assert!(admits(MatchKind::Prefix, 50, 50, common));
    }

    #[test]
    fn stems_fold_accents_and_quoted_phrases_must_match_in_order() {
        let texts = [
            "we scheduled sessions for Monday",
            "the Café dashboard is slow",
            "schedule it like a cron job",
            "sessions were scheduled twice",
        ];
        let mut scheduled = matched(&texts, "scheduling", &[]);
        scheduled.sort_unstable();
        assert_eq!(scheduled, [0, 2, 3]);
        let documents = texts.map(|text| vec![text]);
        let cafe = search_texts(&documents, "cafe", &[]).unwrap();
        assert_eq!(cafe[0].snippet, "the **Café** dashboard is slow");
        assert_eq!(matched(&texts, "\"scheduled sessions\"", &[]), [0]);
        assert!(search_texts(&documents, "  ", &[]).is_err());
    }

    #[test]
    fn a_misspelled_rare_word_still_finds_its_text() {
        let texts = [
            "the thumbnail cache is cold",
            "the standup ran long",
            "lunch was great",
        ];
        assert_eq!(matched(&texts, "thumbnial", &[]), [0]);
    }

    #[test]
    fn words_stem_fold_and_expand_to_their_spelling_neighbours() {
        for (word, expected) in [
            ("scheduled", "schedul"),
            ("schedule", "schedul"),
            ("scheduling", "schedul"),
            ("sessions", "session"),
            ("recurring", "recur"),
            ("recur", "recur"),
            ("automated", "automat"),
            ("dax", "dax"),
            ("class", "class"),
        ] {
            assert_eq!(stem(word), expected, "{word}");
        }
        let vocabulary = [
            "schedul", "settl", "steal", "still", "dack", "deck", "docker",
        ]
        .map(String::from);
        let matches = |term: &str| {
            expand(term, vocabulary.iter())
                .into_iter()
                .map(|(stem, _)| stem)
                .collect::<Vec<_>>()
        };
        // Sound-alikes apply to short, name-like words only.
        assert_eq!(matches("schedul"), ["schedul"]);
        assert_eq!(matches("dax"), ["dack", "deck"]);
        assert_eq!(soundex("dax"), soundex("dack"));
        assert_ne!(soundex("dax"), soundex("max"));
        assert_eq!(fold("Café DAX"), "cafe dax");
        assert_eq!(edit_distance("cron", "corn", 1), Some(1));
        assert_eq!(edit_distance("cron", "crane", 1), None);
    }

    fn entry(term: &str, sounds_like: &[&str]) -> VocabularyEntry {
        VocabularyEntry {
            term: term.into(),
            sounds_like: sounds_like.iter().map(|&text| text.into()).collect(),
        }
    }

    fn matched(texts: &[&str], query: &str, vocabulary: &[VocabularyEntry]) -> Vec<usize> {
        let documents = texts.iter().map(|text| vec![*text]).collect::<Vec<_>>();
        search_texts(&documents, query, vocabulary)
            .unwrap()
            .into_iter()
            .map(|hit| hit.index)
            .collect()
    }

    #[test]
    fn a_vocabulary_term_finds_its_mis_transcriptions_below_exact_hits() {
        let texts = [
            "the cappuccino guides cover release notes and a long list of other things",
            "capuchin capuchin capuchin",
            "lunch was great",
            "cappuccino has the skills",
        ];
        let vocabulary = [entry("cappuccino", &[])];
        // Without the vocabulary, `capuchin` is too far from `cappuccino` to match.
        assert_eq!(matched(&texts, "cappuccino", &[]), [3, 0]);
        // With it, `capuchin` sounds alike, yet ranks after every exact hit even
        // though its repeated word scores higher.
        assert_eq!(matched(&texts, "cappuccinos", &vocabulary), [3, 0, 1]);
        let documents = texts.map(|text| vec![text]);
        let hits = search_texts(&documents, "cappuccino", &vocabulary).unwrap();
        assert_eq!(hits[2].snippet, "**capuchin** **capuchin** **capuchin**");
    }

    #[test]
    fn sounds_like_spellings_and_split_words_alias_a_term() {
        let texts = [
            "the open coat release went out",
            "opencode beta shipped",
            "we should open code review to everyone",
            "the coat is open",
        ];
        let vocabulary = [entry("OpenCode", &["open coat"])];
        assert_eq!(matched(&texts, "opencode", &[]), [1]);
        let hits = matched(&texts, "OpenCode", &vocabulary);
        assert_eq!(hits[0], 1);
        let mut aliased = hits[1..].to_vec();
        aliased.sort_unstable();
        assert_eq!(aliased, [0, 2], "only adjacent words alias the term");
    }

    #[test]
    fn vocabulary_never_pulls_in_common_words() {
        // `Rune` has one consonant sound, so sound-alikes come only from its
        // "sounds like" spellings: the vocabulary changes nothing here.
        let chatter = [
            "rune ships the new layout",
            "I went for a run",
            "the rain stopped",
            "the room is booked",
            "ruin the surprise",
            "rain again tomorrow",
        ];
        assert_eq!(
            matched(&chatter, "rune", &[entry("Rune", &[])]),
            matched(&chatter, "rune", &[])
        );
        // `scroll` sounds like `Scrawl`; once it is a common word here, it is
        // never treated as a mis-transcription.
        let mut texts = vec!["scrawl captures the page", "I fixed the scroll bug"];
        let vocabulary = [entry("Scrawl", &[])];
        assert_eq!(matched(&texts, "scrawl", &vocabulary), [0, 1]);
        texts.extend(["scroll down"; 5]);
        texts.extend(["the standup ran long"; 20]);
        assert_eq!(matched(&texts, "scrawl", &vocabulary), [0]);
        // A spelling the person wrote is always used.
        let written = [entry("Scrawl", &["scroll"])];
        assert_eq!(matched(&texts, "scrawl", &written).len(), 7);
        assert_eq!(matched(&texts, "scrawl", &written)[0], 0);
    }

    #[test]
    fn sound_alikes_need_two_consonant_sounds_and_few_changes() {
        assert_eq!(sound_key("cappuccino"), sound_key("capuchin"));
        assert!(sounds_alike("cappuccino", "capuchin"));
        assert!(sounds_alike("claud", "cloud"));
        assert!(sounds_alike("scrawl", "scroll"));
        // One consonant sound matches too many words.
        assert!(!sounds_alike("rune", "rain"));
        assert!(!sounds_alike("dax", "docks"));
        // The first letter and most letters must agree.
        assert!(!sounds_alike("effect", "affect"));
        assert!(!sounds_alike("parakeet", "pirate"));
        assert!(!sounds_alike("capuchin", "capuchin"));
        // Soundex itself is unchanged by the uncapped key.
        assert_eq!(soundex("cappuccino").as_deref(), Some("c125"));
        assert_eq!(soundex("dax").as_deref(), Some("d200"));
        assert_eq!(soundex("moonshine").as_deref(), Some("m525"));
    }
}

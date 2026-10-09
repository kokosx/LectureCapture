//! Transcript model: word-level timestamps, merging of overlapping chunks and
//! assignment of speech to slide occurrences.
//!
//! Invariant tested below: every transcribed word ends up in exactly one place of the
//! by-slide view (no loss, no duplication), in chronological order, even when an
//! utterance spans a slide change.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Word {
    /// Start / end in ms since lecture start.
    pub s: u64,
    pub e: u64,
    pub w: String,
}

impl Word {
    pub fn mid(&self) -> u64 {
        (self.s + self.e) / 2
    }
}

/// One line of `transcript/segments.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SegmentRecord {
    pub chunk_id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub words: Vec<Word>,
    pub lang: Option<String>,
    pub no_speech_prob: f32,
    pub model: String,
}

/// Raw engine output, times relative to the chunk start.
#[derive(Clone, Debug)]
pub struct RawToken {
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct RawSegment {
    pub t0_ms: u64,
    pub t1_ms: u64,
    pub text: String,
    pub tokens: Vec<RawToken>,
    pub no_speech_prob: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChunkMeta {
    pub chunk_id: u64,
    pub start_ms: u64,
    pub keep_from_ms: u64,
    pub end_ms: u64,
}

fn is_special(t: &str) -> bool {
    let t = t.trim();
    t.starts_with("[_") || t.starts_with("<|") || t.is_empty()
}

/// Group sub-word tokens into words (a token starting with a space starts a new word).
pub fn build_words(seg: &RawSegment) -> Vec<(u64, u64, String)> {
    let mut words: Vec<(u64, u64, String)> = Vec::new();
    let usable: Vec<&RawToken> = seg.tokens.iter().filter(|t| !is_special(&t.text)).collect();
    let has_times = usable.iter().any(|t| t.t1_ms > 0);
    if has_times {
        for t in usable {
            let starts_word = t.text.starts_with(' ') || words.is_empty();
            let piece = t.text.trim_start();
            let attach = !starts_word || piece.starts_with([',', '.', ';', ':', '!', '?', ')', '…']);
            if attach && !words.is_empty() {
                let last = words.last_mut().unwrap();
                last.2.push_str(piece);
                last.1 = last.1.max(t.t1_ms);
            } else {
                words.push((t.t0_ms, t.t1_ms.max(t.t0_ms), piece.to_string()));
            }
        }
    } else {
        // no token timing: spread words over the segment proportionally to length
        let parts: Vec<&str> = seg.text.split_whitespace().collect();
        let total: usize = parts.iter().map(|p| p.chars().count().max(1)).sum();
        let span = seg.t1_ms.saturating_sub(seg.t0_ms);
        let mut acc = 0usize;
        for p in parts {
            let n = p.chars().count().max(1);
            let s = seg.t0_ms + (span as u128 * acc as u128 / total.max(1) as u128) as u64;
            acc += n;
            let e = seg.t0_ms + (span as u128 * acc as u128 / total.max(1) as u128) as u64;
            words.push((s, e, p.to_string()));
        }
    }
    // keep timestamps monotonic and inside the segment
    let mut prev_s = seg.t0_ms;
    for w in &mut words {
        w.0 = w.0.clamp(prev_s, seg.t1_ms.max(prev_s));
        w.1 = w.1.clamp(w.0, seg.t1_ms.max(w.0));
        prev_s = w.0;
    }
    words.retain(|w| !w.2.trim().is_empty());
    words
}

fn norm(w: &str) -> String {
    w.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// Typical whisper hallucinations on silence / music (Polish YouTube subtitles…).
fn is_hallucination(text: &str, no_speech_prob: f32) -> bool {
    let t = text.to_lowercase();
    let t = t.trim();
    if t.contains("amara.org") || t.contains("napisy wykonane przez") || t.contains("napisy stworzone przez") {
        return true;
    }
    let only_brackets = t.starts_with('[') && t.ends_with(']') || t.starts_with('(') && t.ends_with(')');
    if only_brackets || t.chars().all(|c| !c.is_alphanumeric()) {
        return true;
    }
    let generic = [
        "dziękuję za uwagę.",
        "dzięki za obejrzenie!",
        "dziękuję za obejrzenie.",
        "thank you for watching.",
        "thanks for watching!",
        "subskrybuj kanał.",
    ];
    no_speech_prob > 0.5 && generic.contains(&t)
}

pub fn join_words(words: &[Word]) -> String {
    let mut out = String::new();
    for w in words {
        if !out.is_empty() && !w.w.starts_with([',', '.', ';', ':', '!', '?', ')', '…']) {
            out.push(' ');
        }
        out.push_str(&w.w);
    }
    out
}

/// Convert engine output for one chunk into timeline-absolute records, dropping words
/// that belong to the overlap with the previous chunk and obvious hallucinations.
pub fn finalize_chunk(meta: &ChunkMeta, segs: &[RawSegment], lang: Option<String>, model: &str) -> Vec<SegmentRecord> {
    let mut out: Vec<SegmentRecord> = Vec::new();
    for seg in segs {
        if is_hallucination(&seg.text, seg.no_speech_prob) {
            continue;
        }
        let words: Vec<Word> = build_words(seg)
            .into_iter()
            .map(|(s, e, w)| Word {
                s: (meta.start_ms + s).min(meta.end_ms),
                e: (meta.start_ms + e).min(meta.end_ms),
                w,
            })
            .filter(|w| w.mid() >= meta.keep_from_ms)
            .collect();
        if words.is_empty() {
            continue;
        }
        let text = join_words(&words);
        // whisper repetition loop: identical consecutive segment text
        if out.last().is_some_and(|p| norm(&p.text) == norm(&text)) {
            continue;
        }
        out.push(SegmentRecord {
            chunk_id: meta.chunk_id,
            start_ms: words[0].s,
            end_ms: words.last().unwrap().e,
            text,
            words,
            lang: lang.clone(),
            no_speech_prob: seg.no_speech_prob,
            model: model.to_string(),
        });
    }
    out
}

/// Sort records chronologically and remove words repeated across a chunk boundary
/// (the same phrase recognized at the end of chunk N and start of chunk N+1).
pub fn merge(records: &[SegmentRecord]) -> Vec<SegmentRecord> {
    let mut recs: Vec<SegmentRecord> = records.to_vec();
    recs.sort_by_key(|r| (r.start_ms, r.chunk_id));
    // drop exact duplicates (same chunk transcribed twice after a crash)
    recs.dedup_by(|b, a| a.chunk_id == b.chunk_id && a.start_ms == b.start_ms && a.text == b.text);
    let mut out: Vec<SegmentRecord> = Vec::with_capacity(recs.len());
    for mut r in recs {
        if let Some(prev) = out.iter().rev().find(|p| p.chunk_id != r.chunk_id) {
            if prev.chunk_id < r.chunk_id && !prev.words.is_empty() && !r.words.is_empty() {
                let last_e = prev.words.last().unwrap().e;
                if r.words[0].s <= last_e + 1_500 {
                    let tail: Vec<String> = prev.words.iter().rev().take(8).rev().map(|w| norm(&w.w)).collect();
                    let head: Vec<String> = r.words.iter().take(8).map(|w| norm(&w.w)).collect();
                    let mut best = 0;
                    for m in 1..=tail.len().min(head.len()) {
                        if tail[tail.len() - m..] == head[..m] {
                            let long_enough = m >= 2 || head[0].chars().count() >= 4;
                            if long_enough {
                                best = m;
                            }
                        }
                    }
                    if best > 0 {
                        r.words.drain(..best);
                        if r.words.is_empty() {
                            continue;
                        }
                        r.start_ms = r.words[0].s;
                        r.text = join_words(&r.words);
                    }
                }
            }
        }
        out.push(r);
    }
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub start_ms: u64,
    pub end_ms: u64,
    pub words: Vec<Word>,
    /// The utterance started before this part (on the previous slide).
    pub continues_from_prev: bool,
    /// The utterance continues on the next slide.
    pub continues_to_next: bool,
}

impl Part {
    pub fn text(&self) -> String {
        join_words(&self.words)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SlideSpeech {
    pub occurrence_id: String,
    pub slide_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Assignment {
    pub slides: Vec<SlideSpeech>,
    /// Speech before the first slide appeared.
    pub before_first: Vec<Part>,
    /// Speech while no slide was shown (pause, lost video).
    pub outside: Vec<Part>,
}

impl Assignment {
    pub fn word_count(&self) -> usize {
        let c = |p: &Vec<Part>| p.iter().map(|x| x.words.len()).sum::<usize>();
        self.slides.iter().map(|s| c(&s.parts)).sum::<usize>() + c(&self.before_first) + c(&self.outside)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target {
    Before,
    Outside,
    Slide(usize),
}

/// Assign each word to the slide occurrence visible at the word's midpoint.
/// `timeline`: (occurrence id, slide id, start, end), sorted by start.
pub fn assign(records: &[SegmentRecord], timeline: &[(String, u32, u64, u64)]) -> Assignment {
    let mut a = Assignment {
        slides: timeline
            .iter()
            .map(|(id, sid, s, e)| SlideSpeech {
                occurrence_id: id.clone(),
                slide_id: *sid,
                start_ms: *s,
                end_ms: *e,
                parts: Vec::new(),
            })
            .collect(),
        ..Default::default()
    };
    let target_of = |t: u64| -> Target {
        let idx = timeline.partition_point(|(_, _, s, _)| *s <= t);
        if idx == 0 {
            return if timeline.is_empty() { Target::Outside } else { Target::Before };
        }
        let (_, _, _, e) = &timeline[idx - 1];
        if t < *e {
            Target::Slide(idx - 1)
        } else {
            Target::Outside
        }
    };
    for r in records {
        let mut groups: Vec<(Target, Vec<Word>)> = Vec::new();
        for w in &r.words {
            let t = target_of(w.mid());
            match groups.last_mut() {
                Some((gt, ws)) if *gt == t => ws.push(w.clone()),
                _ => groups.push((t, vec![w.clone()])),
            }
        }
        let n = groups.len();
        for (i, (t, words)) in groups.into_iter().enumerate() {
            let part = Part {
                start_ms: words[0].s,
                end_ms: words.last().unwrap().e,
                words,
                continues_from_prev: i > 0,
                continues_to_next: i + 1 < n,
            };
            match t {
                Target::Before => a.before_first.push(part),
                Target::Outside => a.outside.push(part),
                Target::Slide(k) => a.slides[k].parts.push(part),
            }
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(t0: u64, t1: u64, s: &str) -> RawToken {
        RawToken { t0_ms: t0, t1_ms: t1, text: s.into() }
    }

    fn rec(chunk: u64, words: &[(u64, u64, &str)]) -> SegmentRecord {
        let ws: Vec<Word> = words.iter().map(|&(s, e, w)| Word { s, e, w: w.into() }).collect();
        SegmentRecord {
            chunk_id: chunk,
            start_ms: ws[0].s,
            end_ms: ws.last().unwrap().e,
            text: join_words(&ws),
            words: ws,
            lang: Some("pl".into()),
            no_speech_prob: 0.0,
            model: "test".into(),
        }
    }

    #[test]
    fn tokens_become_words_with_punctuation_attached() {
        let seg = RawSegment {
            t0_ms: 0,
            t1_ms: 2000,
            text: " Dzień dobry, państwu.".into(),
            tokens: vec![
                tok(0, 200, "[_BEG_]"),
                tok(0, 300, " Dzi"),
                tok(300, 500, "eń"),
                tok(500, 900, " dobry"),
                tok(900, 950, ","),
                tok(1000, 1500, " pa"),
                tok(1500, 1800, "ństwu"),
                tok(1800, 1850, "."),
            ],
            no_speech_prob: 0.01,
        };
        let w = build_words(&seg);
        let texts: Vec<&str> = w.iter().map(|x| x.2.as_str()).collect();
        assert_eq!(texts, vec!["Dzień", "dobry,", "państwu."]);
        assert_eq!((w[0].0, w[0].1), (0, 500));
        assert_eq!((w[2].0, w[2].1), (1000, 1850));
    }

    #[test]
    fn words_without_token_times_are_interpolated() {
        let seg = RawSegment { t0_ms: 1000, t1_ms: 3000, text: "ala ma kota".into(), tokens: vec![], no_speech_prob: 0.0 };
        let w = build_words(&seg);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].0, 1000);
        assert_eq!(w[2].1, 3000);
        assert!(w.windows(2).all(|p| p[0].1 <= p[1].0));
    }

    #[test]
    fn overlap_words_before_keep_from_are_dropped() {
        let meta = ChunkMeta { chunk_id: 2, start_ms: 27_000, keep_from_ms: 28_000, end_ms: 50_000 };
        let seg = RawSegment {
            t0_ms: 0,
            t1_ms: 3000,
            text: "x".into(),
            tokens: vec![tok(0, 400, " koniec"), tok(500, 900, " zdania"), tok(1100, 1500, " nowe"), tok(1600, 2000, " zdanie")],
            no_speech_prob: 0.0,
        };
        let r = finalize_chunk(&meta, &[seg], Some("pl".into()), "base");
        assert_eq!(r[0].text, "nowe zdanie");
        assert_eq!(r[0].start_ms, 28_100);
    }

    #[test]
    fn hallucinations_filtered() {
        let meta = ChunkMeta { chunk_id: 1, start_ms: 0, keep_from_ms: 0, end_ms: 5000 };
        let seg = RawSegment {
            t0_ms: 0,
            t1_ms: 3000,
            text: " Napisy stworzone przez społeczność Amara.org".into(),
            tokens: vec![],
            no_speech_prob: 0.1,
        };
        assert!(finalize_chunk(&meta, &[seg], None, "base").is_empty());
    }

    #[test]
    fn boundary_duplicates_removed_on_merge() {
        let a = rec(1, &[(0, 400, "drzewo"), (500, 900, "binarne"), (1000, 1400, "ma"), (1500, 1900, "dwoje")]);
        let b = rec(2, &[(1450, 1900, "ma"), (1950, 2300, "dwoje"), (2400, 2900, "dzieci")]);
        let m = merge(&[b, a]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].text, "dzieci");
    }

    #[test]
    fn speech_spanning_slide_change_is_split_without_loss_or_duplication() {
        // slide 1: 0–5 s, slide 2: 5–12 s (revisit of slide 1 afterwards)
        let timeline = vec![
            ("occ-0001".to_string(), 1, 1_000, 5_000),
            ("occ-0002".to_string(), 2, 5_000, 12_000),
            ("occ-0003".to_string(), 1, 12_000, 20_000),
        ];
        let r1 = rec(1, &[(200, 600, "Zaczynamy."), (1200, 1600, "To"), (1700, 2000, "jest")]);
        let r2 = rec(2, &[(4000, 4500, "zdanie"), (4600, 4950, "które"), (5050, 5400, "przechodzi"), (5500, 6000, "dalej")]);
        let r3 = rec(3, &[(12_500, 13_000, "Wracamy."), (25_000, 25_500, "Koniec.")]);
        let all = vec![r1, r2, r3];
        let a = assign(&all, &timeline);
        let total: usize = all.iter().map(|r| r.words.len()).sum();
        assert_eq!(a.word_count(), total, "no word lost or duplicated");
        assert_eq!(a.before_first.len(), 1);
        assert_eq!(a.before_first[0].text(), "Zaczynamy.");
        assert_eq!(a.slides[0].parts.last().unwrap().text(), "zdanie które");
        assert!(a.slides[0].parts.last().unwrap().continues_to_next);
        assert_eq!(a.slides[1].parts[0].text(), "przechodzi dalej");
        assert!(a.slides[1].parts[0].continues_from_prev);
        assert_eq!(a.slides[2].parts[0].text(), "Wracamy.");
        assert_eq!(a.outside[0].text(), "Koniec.");
        // chronological order inside the flattened view
        let mut flat: Vec<u64> = a.before_first.iter().flat_map(|p| p.words.iter().map(|w| w.s)).collect();
        for s in &a.slides {
            flat.extend(s.parts.iter().flat_map(|p| p.words.iter().map(|w| w.s)));
        }
        assert!(flat.windows(2).all(|w| w[0] <= w[1]));
    }
}

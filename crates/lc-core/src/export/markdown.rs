//! Human-readable Markdown documents. All links are relative so the folder can be
//! moved between machines.

use super::LectureData;
use crate::session::manifest::*;
use crate::transcript::{join_words, Part, SegmentRecord};
use crate::util::{fmt_duration_long, fmt_ms};
use chrono::Datelike;
use std::fmt::Write;

pub fn weekday_pl(t: &Time) -> &'static str {
    ["poniedziałek", "wtorek", "środa", "czwartek", "piątek", "sobota", "niedziela"]
        [t.weekday().num_days_from_monday() as usize]
}

pub fn gap_label(k: GapKind) -> &'static str {
    match k {
        GapKind::Paused => "nagrywanie wstrzymane (pauza)",
        GapKind::VideoLost => "utracony obraz źródła (okno zamknięte/ukryte lub brak uprawnień)",
        GapKind::AudioLost => "utracone audio (urządzenie/strumień niedostępny)",
        GapKind::NoAudioSignal => "brak sygnału audio (cisza)",
        GapKind::Crash => "nieoczekiwane zakończenie nagrywania",
        GapKind::TranscriptionFailed => "nieudana transkrypcja fragmentu",
    }
}

pub fn transcription_status_label(s: TranscriptionStatus) -> &'static str {
    match s {
        TranscriptionStatus::Disabled => "wyłączona",
        TranscriptionStatus::Pending => "oczekuje",
        TranscriptionStatus::Running => "w toku",
        TranscriptionStatus::Completed => "zakończona",
        TranscriptionStatus::Partial => "częściowa (niektóre fragmenty nie zostały rozpoznane)",
        TranscriptionStatus::Failed => "nieudana",
    }
}

fn range(a: u64, b: u64) -> String {
    format!("{}–{}", fmt_ms(a), fmt_ms(b))
}

fn parts_text(parts: &[Part]) -> String {
    let mut out = String::new();
    for p in parts {
        if !out.is_empty() {
            out.push('\n');
        }
        let pre = if p.continues_from_prev { "… " } else { "" };
        let post = if p.continues_to_next { " …" } else { "" };
        let _ = write!(out, "[{}] {pre}{}{post}", fmt_ms(p.start_ms), p.text());
    }
    out
}

/// Split records into readable paragraphs (pause > 2.5 s or > 90 s per paragraph).
fn paragraphs(records: &[SegmentRecord]) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut cur: Option<(u64, u64, Vec<String>)> = None;
    for r in records {
        let text = join_words(&r.words);
        match &mut cur {
            Some((start, end, texts)) if r.start_ms <= *end + 2_500 && r.start_ms < *start + 90_000 => {
                texts.push(text);
                *end = r.end_ms;
            }
            _ => {
                if let Some((s, _, t)) = cur.take() {
                    out.push((s, t.join(" ")));
                }
                cur = Some((r.start_ms, r.end_ms, vec![text]));
            }
        }
    }
    if let Some((s, _, t)) = cur {
        out.push((s, t.join(" ")));
    }
    out
}

fn header_table(m: &Manifest) -> String {
    let l = &m.lecture;
    let mut s = String::new();
    let _ = writeln!(s, "| | |\n|---|---|");
    let _ = writeln!(s, "| Data | {} ({}) |", l.started_at.format("%Y-%m-%d"), weekday_pl(&l.started_at));
    let _ = writeln!(s, "| Rozpoczęcie | {} |", l.started_at.format("%H:%M:%S"));
    if let Some(e) = &l.ended_at {
        let _ = writeln!(s, "| Zakończenie | {} |", e.format("%H:%M:%S"));
    }
    let _ = writeln!(s, "| Czas trwania | {} |", fmt_duration_long(m.end_ms()));
    let _ = writeln!(s, "| Liczba slajdów | {} (wyświetleń na osi czasu: {}) |", m.slides.len(), m.timeline.len());
    let t = &m.transcription;
    if t.enabled {
        let _ = writeln!(
            s,
            "| Transkrypcja | {} · model `{}` · język `{}` · status: {} |",
            t.engine,
            t.model,
            t.language,
            transcription_status_label(t.status)
        );
    } else {
        let _ = writeln!(s, "| Transkrypcja | wyłączona |");
    }
    match &m.audio.file {
        Some(f) => {
            let _ = writeln!(s, "| Nagranie audio | `{f}` (Ogg/Opus, {} kb/s) |", m.audio.bitrate / 1000);
        }
        None => {
            let _ = writeln!(s, "| Nagranie audio | nie zachowano (usunięte po transkrypcji) |");
        }
    }
    let src = &m.capture.source;
    let _ = writeln!(
        s,
        "| Źródło obrazu | {} {} |",
        src.kind,
        [src.app_name.clone(), src.title.clone()].into_iter().flatten().collect::<Vec<_>>().join(" — ")
    );
    if l.status == LectureStatus::Recovered {
        let _ = writeln!(s, "| Status | odzyskany po nieoczekiwanym zakończeniu |");
    }
    s
}

fn gaps_section(m: &Manifest, data: &LectureData) -> String {
    let mut s = String::from("## Braki i jakość danych\n\n");
    let mut any = false;
    for g in &m.gaps {
        any = true;
        let end = g.end_ms.unwrap_or(m.end_ms());
        let detail = g.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default();
        let _ = writeln!(s, "- {} — {}{}", range(g.start_ms, end), gap_label(g.kind), detail);
    }
    let t = &m.transcription;
    if t.enabled && t.status != TranscriptionStatus::Completed {
        any = true;
        let _ = writeln!(
            s,
            "- Transkrypcja: {} ({} z {} fragmentów{}).",
            transcription_status_label(t.status),
            t.chunks_done,
            t.chunks_total,
            if t.chunks_failed > 0 { format!(", nieudane: {}", t.chunks_failed) } else { String::new() }
        );
    }
    if t.enabled && data.records.is_empty() {
        any = true;
        let _ = writeln!(s, "- Brak rozpoznanych wypowiedzi w transkrypcji.");
    }
    if !data.assignment.outside.is_empty() {
        any = true;
        let _ = writeln!(
            s,
            "- {} fragmentów wypowiedzi przypada na okresy bez wyświetlanego slajdu (sekcja „Poza slajdami”).",
            data.assignment.outside.len()
        );
    }
    for n in &m.lecture.notes {
        any = true;
        let _ = writeln!(s, "- {n}");
    }
    if !any {
        s.push_str("Nie zarejestrowano braków danych.\n");
    }
    s
}

pub fn lecture_md(m: &Manifest, data: &LectureData) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# {}\n", m.lecture.title);
    s.push_str(&header_table(m));
    s.push('\n');
    s.push_str(&gaps_section(m, data));
    s.push_str("\n## Pliki\n\n");
    s.push_str("- `slides/` – zrzuty slajdów (PNG, bezstratnie, natywna rozdzielczość)\n");
    s.push_str("- `transcript/full.md` – pełna transkrypcja chronologicznie\n");
    s.push_str("- `transcript/by-slide.md` – transkrypcja podzielona według slajdów\n");
    s.push_str("- `transcript/segments.jsonl` – segmenty z czasami słów (dane maszynowe)\n");
    s.push_str("- `manifest.json` – metadane i pełna oś czasu\n");
    s.push_str("- `PROMPT.md` – instrukcja dla agenta AI do opracowania notatek\n\n");

    s.push_str("## Przebieg wykładu\n\n");
    if !data.assignment.before_first.is_empty() {
        let p = &data.assignment.before_first;
        let _ = writeln!(s, "### [{}] Przed pierwszym slajdem\n", fmt_ms(p[0].start_ms));
        let _ = writeln!(s, "{}\n", quote(&parts_text(p)));
    }
    let mut seen: std::collections::HashMap<u32, u64> = Default::default();
    for (i, sp) in data.assignment.slides.iter().enumerate() {
        let Some(slide) = m.slide(sp.slide_id) else { continue };
        let again = seen.get(&sp.slide_id).map(|t| format!(" (ponownie – wcześniej {})", fmt_ms(*t))).unwrap_or_default();
        seen.entry(sp.slide_id).or_insert(sp.start_ms);
        let _ = writeln!(s, "### [{}] Slajd {}{}\n", fmt_ms(sp.start_ms), slide.id, again);
        let _ = writeln!(s, "![Slajd {}]({})\n", slide.id, slide.file);
        let mut meta = format!("*Wyświetlany: {} · wystąpienie `{}`", range(sp.start_ms, sp.end_ms), sp.occurrence_id);
        if slide.trigger == SlideTrigger::Manual {
            meta.push_str(" · zrzut ręczny");
        }
        if let Some(b) = slide.build_of {
            let _ = write!(meta, " · kolejny etap slajdu {b}");
        }
        meta.push('*');
        let _ = writeln!(s, "{meta}\n");
        if sp.parts.is_empty() {
            let _ = writeln!(s, "_(brak wypowiedzi przypisanych do tego slajdu)_\n");
        } else {
            let _ = writeln!(s, "{}\n", quote(&parts_text(&sp.parts)));
        }
        let _ = i;
    }
    let unplaced: Vec<&Slide> = m.slides.iter().filter(|sl| sl.occurrences.is_empty()).collect();
    if !unplaced.is_empty() {
        s.push_str("### Slajdy bez pozycji na osi czasu\n\n");
        for sl in unplaced {
            let _ = writeln!(s, "![Slajd {}]({})\n", sl.id, sl.file);
        }
    }
    if !data.assignment.outside.is_empty() {
        s.push_str("### Poza slajdami\n\n");
        let _ = writeln!(s, "{}\n", quote(&parts_text(&data.assignment.outside)));
    }
    s
}

fn quote(text: &str) -> String {
    text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n>\n")
}

pub fn full_md(m: &Manifest, data: &LectureData) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Transkrypcja — {}\n", m.lecture.title);
    let _ = writeln!(
        s,
        "Data: {} · model: `{}` · język: `{}` · status: {}\n",
        m.lecture.started_at.format("%Y-%m-%d %H:%M"),
        m.transcription.model,
        m.transcription.language,
        transcription_status_label(m.transcription.status)
    );
    s.push_str("> Znaczniki `[MM:SS]` liczone od początku wykładu. Transkrypcja automatyczna (whisper.cpp) – może zawierać błędy rozpoznawania.\n\n");
    if data.records.is_empty() {
        s.push_str("_(brak rozpoznanych wypowiedzi)_\n");
    }
    for (start, text) in paragraphs(&data.records) {
        let _ = writeln!(s, "**[{}]** {}\n", fmt_ms(start), text);
    }
    s
}

pub fn by_slide_md(m: &Manifest, data: &LectureData) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "# Transkrypcja według slajdów — {}\n", m.lecture.title);
    s.push_str("> Każda wypowiedź jest przypisana do slajdu widocznego w chwili jej wygłoszenia. „…” oznacza zdanie kontynuowane na sąsiednim slajdzie.\n\n");
    if !data.assignment.before_first.is_empty() {
        let _ = writeln!(s, "## Przed pierwszym slajdem\n\n{}\n", parts_text(&data.assignment.before_first));
    }
    for sp in &data.assignment.slides {
        let file = m.slide(sp.slide_id).map(|sl| sl.file.clone()).unwrap_or_default();
        let _ = writeln!(s, "## Slajd {} · {} · `{}`\n", sp.slide_id, range(sp.start_ms, sp.end_ms), sp.occurrence_id);
        let _ = writeln!(s, "![Slajd {}](../{})\n", sp.slide_id, file);
        if sp.parts.is_empty() {
            s.push_str("_(brak wypowiedzi)_\n\n");
        } else {
            let _ = writeln!(s, "{}\n", parts_text(&sp.parts));
        }
    }
    if !data.assignment.outside.is_empty() {
        let _ = writeln!(s, "## Poza slajdami\n\n{}\n", parts_text(&data.assignment.outside));
    }
    s
}

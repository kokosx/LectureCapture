//! PROMPT.md – instructions for an AI agent (Claude Code / Claude Cowork / ChatGPT
//! Work …) that is pointed at the lecture folder. Deliberately vendor-neutral: it only
//! relies on the agent being able to read files (including images) and write files.

use super::LectureData;
use crate::session::manifest::*;
use crate::util::{fmt_duration_long, fmt_ms};
use std::fmt::Write;

/// Slides per batch for long lectures.
pub const BATCH_SLIDES: usize = 15;

pub struct Batch {
    pub index: usize,
    pub occurrences: (usize, usize),
    pub slide_ids: Vec<u32>,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Split the timeline into batches of about `BATCH_SLIDES` distinct slides.
pub fn batches(m: &Manifest) -> Vec<Batch> {
    let tl = m.closed_timeline();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tl.len() {
        let mut ids: Vec<u32> = Vec::new();
        let mut j = i;
        while j < tl.len() && (ids.len() < BATCH_SLIDES || ids.contains(&tl[j].1)) {
            if !ids.contains(&tl[j].1) {
                ids.push(tl[j].1);
            }
            j += 1;
        }
        out.push(Batch {
            index: out.len() + 1,
            occurrences: (i, j - 1),
            slide_ids: ids,
            start_ms: if out.is_empty() { 0 } else { tl[i].2 },
            end_ms: if j == tl.len() { m.end_ms().max(tl[j - 1].3) } else { tl[j - 1].3 },
        });
        i = j;
    }
    out
}

fn ids_list(ids: &[u32]) -> String {
    ids.iter().map(|i| format!("{i:03}")).collect::<Vec<_>>().join(", ")
}

pub fn prompt_md(m: &Manifest, data: &LectureData) -> String {
    let l = &m.lecture;
    let words = data.records.iter().map(|r| r.words.len()).sum::<usize>();
    let batches = batches(m);
    let long = m.slides.len() > BATCH_SLIDES + 5 || words > 12_000;
    let mut s = String::new();

    let _ = writeln!(s, "# PROMPT — opracowanie notatek z wykładu „{}”\n", l.title);
    s.push_str(
        "Jesteś asystentem naukowym. Ten folder zawiera kompletny zapis wykładu przygotowany \
automatycznie przez aplikację LectureCapture: zrzuty wszystkich slajdów, pełną automatyczną \
transkrypcję wypowiedzi prowadzącego i oś czasu łączącą jedno z drugim. Twoim zadaniem jest \
przygotowanie profesjonalnych, rzetelnych materiałów do nauki **w języku polskim**.\n\n",
    );

    s.push_str("## Dane wykładu\n\n");
    let _ = writeln!(s, "- Tytuł: **{}**", l.title);
    let _ = writeln!(s, "- Data: {} ({})", l.started_at.format("%Y-%m-%d %H:%M"), super::markdown::weekday_pl(&l.started_at));
    let _ = writeln!(s, "- Czas trwania: {}", fmt_duration_long(m.end_ms()));
    let _ = writeln!(s, "- Liczba zapisanych slajdów: **{}** (wyświetleń na osi czasu: {})", m.slides.len(), m.timeline.len());
    let _ = writeln!(
        s,
        "- Transkrypcja: {} słów, model `{}`, język `{}`, status: {}",
        words,
        m.transcription.model,
        m.transcription.language,
        super::markdown::transcription_status_label(m.transcription.status)
    );
    if !m.gaps.is_empty() || !l.notes.is_empty() {
        s.push_str("- Uwaga: w materiale występują braki danych – szczegóły w sekcji „Braki i jakość danych” w `lecture.md`.\n");
    }
    s.push('\n');

    s.push_str("## Pliki w folderze\n\n");
    s.push_str("| Plik | Zawartość |\n|---|---|\n");
    s.push_str("| `manifest.json` | metadane, lista slajdów (`slides`), oś czasu (`timeline`: który slajd był widoczny od–do), braki danych (`gaps`) |\n");
    s.push_str("| `lecture.md` | przegląd wykładu: każdy slajd z obrazem i przypisanymi wypowiedziami |\n");
    s.push_str("| `slides/NNN.png` | zrzuty slajdów w natywnej rozdzielczości (bezstratnie); numer pliku = numer slajdu |\n");
    s.push_str("| `transcript/full.md` | **pełna** transkrypcja chronologicznie, niezależnie od slajdów |\n");
    s.push_str("| `transcript/by-slide.md` | ta sama transkrypcja podzielona według slajdów |\n");
    s.push_str("| `transcript/segments.jsonl` | segmenty z czasami pojedynczych słów (gdy potrzebujesz precyzji) |\n\n");
    s.push_str("Czasy `[MM:SS]` / `*_ms` są liczone od początku wykładu. Ten sam slajd może pojawić się na osi czasu kilka razy (prowadzący wrócił do niego) – wtedy używa tego samego pliku PNG.\n\n");

    s.push_str("## Procedura (wykonaj wszystkie kroki)\n\n");
    s.push_str("1. **Przeczytaj `manifest.json` i `lecture.md`**, żeby poznać strukturę wykładu, listę slajdów, oś czasu i ewentualne braki danych.\n");
    let _ = writeln!(
        s,
        "2. **Przeanalizuj WSZYSTKIE {} slajdy jako obrazy.** Otwórz i obejrzyj każdy plik `slides/NNN.png` – odczytaj tekst, wzory, tabele, wykresy, diagramy i kod. Nie wnioskuj o treści z nazw plików ani z samej transkrypcji. Jeśli fragment slajdu jest nieczytelny (niska rozdzielczość, rozmycie), napisz to wprost zamiast zgadywać.",
        m.slides.len()
    );
    s.push_str("3. **Przeczytaj pełną transkrypcję** (`transcript/full.md` – cały plik, nie wybrane fragmenty) oraz `transcript/by-slide.md`, aby wiedzieć, co prowadzący mówił przy którym slajdzie.\n");
    s.push_str("4. **Połącz treść slajdów z komentarzem prowadzącego.** Szczególnie wartościowe są dodatkowe wyjaśnienia, przykłady, intuicje, ostrzeżenia („to będzie na egzaminie”, „częsty błąd”) i dygresje, których nie ma na slajdach – uwzględnij je i oznacz jako *(z wykładu)*.\n");
    s.push_str("5. **Zidentyfikuj** zagadnienia, definicje, twierdzenia, wzory, algorytmy (z krokami i złożonością, jeśli dotyczy), przykłady i zależności między pojęciami.\n");
    s.push_str("6. **Napisz kompleksowe notatki po polsku**, zachowując poprawną terminologię naukową i techniczną. Angielskie terminy podawaj w nawiasie przy pierwszym użyciu, jeśli padły na wykładzie lub są na slajdach. Wzory zapisuj w LaTeX (`$...$`, `$$...$$`), kod w blokach kodu.\n");
    s.push_str("7. **Trudne zagadnienia wyjaśnij prostszym językiem** (np. akapit „Intuicyjnie:” lub prosty przykład), ale bez utraty precyzji.\n");
    s.push_str("8. **Podawaj źródło**: przy każdym zagadnieniu numer(y) slajdów, np. *(slajd 7)*, *(slajdy 12–14)*, a dla treści tylko z wypowiedzi – znacznik czasu, np. *(wykład 23:15)*.\n\n");

    s.push_str("## Zasady rzetelności\n\n");
    s.push_str("- **Nie wymyślaj informacji**, których nie da się ustalić na podstawie slajdów i transkrypcji. Wiedzę spoza wykładu dodawaj tylko, gdy jest niezbędna do zrozumienia, i oznacz ją wyraźnie jako *[uzupełnienie spoza wykładu]*.\n");
    s.push_str("- Transkrypcja jest automatyczna: może zawierać błędnie rozpoznane słowa (zwłaszcza nazwiska, terminy, liczby). Gdy slajd i transkrypcja się różnią, zaufaj slajdowi; gdy sens jest niepewny, zaznacz to.\n");
    s.push_str("- Oznaczaj problemy w tekście: `[niejasne: …]` (niepewny sens), `[niesłyszalne]` (brak/zniekształcone audio), `[niekompletne: …]` (urwany wątek, brak slajdu), `[nieczytelne na slajdzie N]`.\n");
    s.push_str("- Uwzględnij braki danych opisane w `lecture.md` (pauzy, utrata obrazu lub dźwięku) – wspomnij o nich w notatkach w odpowiednich miejscach.\n");
    s.push_str("- Niczego nie pomijaj: każdy slajd musi zostać omówiony lub świadomie oznaczony jako nieistotny (np. slajd tytułowy, organizacyjny).\n\n");

    s.push_str("## Pliki do utworzenia (w tym folderze)\n\n");
    s.push_str("1. **`notes.md`** – pełne notatki z wykładu: spis treści, sekcje według logicznej struktury wykładu (nie mechanicznie slajd po slajdzie), definicje, wzory, algorytmy, przykłady, wyjaśnienia prowadzącego, odwołania do slajdów; możesz osadzać ważne slajdy jako obrazy `![Slajd 7](slides/007.png)`.\n");
    s.push_str("2. **`summary.md`** – zwięzłe podsumowanie (1–2 strony) oraz wyraźna sekcja **„Najważniejsze do zapamiętania”** (lista kluczowych faktów, definicji i wzorów).\n");
    s.push_str("3. **`exam-questions.md`** – lista potencjalnych pytań egzaminacyjnych (otwarte, testowe, zadania obliczeniowe – zależnie od przedmiotu) **z pełnymi odpowiedziami** i odwołaniem do slajdów. Uporządkuj od podstawowych do trudnych.\n");
    s.push_str("4. **`flashcards.md`** – zestaw fiszek do nauki w formacie:\n\n");
    s.push_str("   ```\n   ### <krótki temat>\n   **P:** pytanie\n   **O:** odpowiedź (zwięzła) *(slajd N)*\n   ```\n\n");
    s.push_str("   Dodatkowo możesz utworzyć `flashcards.csv` (kolumny: `pytanie;odpowiedź;tag`) do importu w Anki.\n\n");

    if long {
        s.push_str("## Przetwarzanie partiami (długi wykład)\n\n");
        s.push_str("Materiał jest długi – przetwarzaj go **partiami w podanej kolejności**, nie pomijając żadnej. Dla każdej partii: obejrzyj wszystkie jej slajdy, przeczytaj odpowiadający jej fragment transkrypcji (sekcje w `transcript/by-slide.md` dla tych slajdów oraz zakres czasowy w `transcript/full.md`) i **dopisz** wynik do `notes.md`. Po każdej partii zaktualizuj plik `notes-progress.md` (lista partii z ✅). Gdy kontekst się kończy, kontynuuj od pierwszej nieodhaczonej partii. Dopiero po ukończeniu wszystkich partii przeczytaj całe `notes.md`, ujednolić strukturę i przygotuj `summary.md`, `exam-questions.md` i `flashcards.md`.\n\n");
        s.push_str("| Partia | Slajdy | Zakres czasu |\n|---|---|---|\n");
        for b in &batches {
            let _ = writeln!(s, "| {} | {} | {}–{} |", b.index, ids_list(&b.slide_ids), fmt_ms(b.start_ms), fmt_ms(b.end_ms));
        }
        s.push('\n');
    }

    s.push_str("## Kontrola końcowa\n\n");
    let _ = writeln!(
        s,
        "- [ ] Obejrzałem wszystkie {} obrazy slajdów (lista w `manifest.json` → `slides`).",
        m.slides.len()
    );
    s.push_str("- [ ] Przeczytałem całą transkrypcję od początku do końca.\n");
    s.push_str("- [ ] Każde zagadnienie ma odwołanie do slajdu lub czasu wypowiedzi.\n");
    s.push_str("- [ ] Niejasności i braki są oznaczone, nic nie zostało zmyślone.\n");
    s.push_str("- [ ] Utworzyłem `notes.md`, `summary.md`, `exam-questions.md`, `flashcards.md`.\n\n");
    let _ = writeln!(
        s,
        "_Wygenerowano automatycznie przez LectureCapture {} · {} · nagranie {}–{}._",
        crate::APP_VERSION,
        l.started_at.format("%Y-%m-%d"),
        fmt_ms(0),
        fmt_ms(m.end_ms())
    );
    s
}

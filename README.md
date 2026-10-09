# LectureCapture

**Lokalny rejestrator wykładów z Microsoft Teams** (macOS, Windows). Kliknij *Start*, a aplikacja przez kilka godzin
sama zapisuje każdy nowy slajd jako bezstratny PNG, nagrywa dźwięk wykładowcy, transkrybuje go **offline**
(whisper.cpp z akceleracją Metal), synchronizuje wypowiedzi ze slajdami i na koniec tworzy uporządkowany folder
z gotowym `PROMPT.md` dla agenta AI (Claude Code, Claude Cowork, ChatGPT…), który zrobi z tego notatki,
podsumowanie, pytania egzaminacyjne i fiszki.

Bez chmury, kont, kluczy API i telemetrii. Jedyne połączenie sieciowe: pobranie wybranego modelu Whisper (na żądanie, z weryfikacją SHA‑256).

---

## Co dostajesz po wykładzie

```
2026-10-09_Algorytmy-i-struktury-danych/
├── slides/001.png 002.png …      # każdy slajd raz, natywna rozdzielczość, PNG bezstratnie
├── audio/recording.ogg           # Opus 16 kHz mono (opcjonalnie usuwany po transkrypcji)
├── transcript/
│   ├── full.md                   # pełna transkrypcja z czasami [MM:SS]
│   ├── by-slide.md               # wypowiedzi przypisane do slajdów
│   └── segments.jsonl            # segmenty + czasy pojedynczych słów (dane maszynowe)
├── manifest.json                 # schemat v1: slajdy, oś czasu, braki danych, zdarzenia
├── lecture.md                    # przegląd: slajd → obraz → co mówił prowadzący
└── PROMPT.md                     # instrukcja dla agenta AI
```

### Jak zrobić materiały do nauki z Claude

1. W aplikacji otwórz wykład → **Kopiuj prompt** (albo po prostu otwórz `PROMPT.md`).
2. Uruchom Claude Code / Claude Cowork **w folderze wykładu** (np. `cd …/2026-10-09_Algorytmy && claude`) i wklej prompt
   – albo napisz: *„Przeczytaj PROMPT.md i wykonaj zadanie.”*
3. Agent przeanalizuje **wszystkie obrazy slajdów** i **całą transkrypcję**, a następnie utworzy
   `notes.md`, `summary.md`, `exam-questions.md` i `flashcards.md` (+ opcjonalnie `flashcards.csv` do Anki).
   Dla długich wykładów prompt zawiera plan partii (np. po 15 slajdów) z checklistą postępu.

Prompt wymaga m.in.: odwołań do numerów slajdów i czasu wypowiedzi, oznaczania niejasnych/niesłyszalnych fragmentów,
niewymyślania treści, terminologii fachowej po polsku i prostszych wyjaśnień trudnych zagadnień.

---

## Status funkcji (co jest naprawdę przetestowane)

| Funkcja | Status |
|---|---|
| Detekcja zmiany slajdu (kursor, szum kompresji, przesunięcia 1 px, maski, stabilność, timeout dla animacji) | ✅ testy automatyczne (syntetyczne slajdy) |
| Stopniowe ujawnianie punktów (scalanie lub osobne slajdy), deduplikacja i powroty do slajdów | ✅ testy automatyczne |
| Zapis PNG bit‑w‑bit bezstratnie w natywnej rozdzielczości, SHA‑256 | ✅ testy automatyczne |
| Resampling 48/44,1 kHz → 16 kHz, mikser źródeł zsynchronizowany z zegarem, luki (uśpienie) bez alokacji | ✅ testy automatyczne |
| VAD, dzielenie długiej mowy z nakładką, deduplikacja słów na granicach | ✅ testy automatyczne |
| Ogg/Opus strumieniowo, odczyt uciętego pliku po awarii | ✅ testy automatyczne |
| Synchronizacja słów ze slajdami (zdanie przez zmianę slajdu: bez gubienia i duplikowania słów) | ✅ testy automatyczne |
| Pauza (dokładnie na osi czasu), Stop w trakcie mowy, brak sygnału audio, błędy transkrypcji, brak modelu | ✅ test integracyjny całego pipeline'u |
| Odzyskiwanie po awarii + wznowienie transkrypcji | ✅ test integracyjny |
| Markdown / JSON / PROMPT.md / ZIP, edycje (usuwanie slajdu, przesuwanie granic) | ✅ testy automatyczne |
| Transkrypcja whisper.cpp (Metal) – prawdziwa polska i angielska mowa, pełny pipeline z prawdziwym modelem | ✅ testy z modelem `base` na MacBook (M‑series) |
| Menedżer modeli: pobieranie z Hugging Face, weryfikacja SHA‑256, postęp | ✅ sprawdzone (tiny, base) |
| Przechwytywanie ScreenCaptureKit (okno/monitor/obszar) + dźwięk systemowy na macOS | ⏳ patrz sekcja „Weryfikacja na urządzeniu” |
| Windows: Windows Graphics Capture + WASAPI loopback | ⚠️ zaimplementowane, kompiluje się; testy rdzenia uruchamiane w CI na Windows, **nie zweryfikowane na fizycznym Windowsie** |

## Instalacja

### Gotowe instalatory
Wydania (DMG dla Apple Silicon, instalator NSIS dla Windows) budowane są przez GitHub Actions dla tagów `v*`
– zakładka *Releases*. Modele Whisper nie są dołączane (pobierasz je w aplikacji: *Modele Whisper*).

### Budowanie ze źródeł (macOS)

Wymagania: macOS 15+, Xcode Command Line Tools, Rust (stable), Node.js 20+, CMake (`brew install cmake`).

```bash
git clone https://github.com/kokosx/LectureCapture && cd LectureCapture
npm install
npx tauri build --bundles app        # → target/release/bundle/macos/LectureCapture.app
```

Tryb deweloperski: `npx tauri dev`. Dla stabilnych uprawnień TCC podpisz aplikację swoim certyfikatem:
`APPLE_SIGNING_IDENTITY="Apple Development: …" npx tauri build`.

### Windows
Wymagania: Rust (MSVC), Node.js, CMake, Visual Studio Build Tools. `npm install && npx tauri build`.

## Pierwsze uruchomienie

1. Potwierdź informację o zasadach nagrywania (zgoda prowadzącego/uczelni).
2. macOS: nadaj uprawnienie **Nagrywanie ekranu i dźwięku systemowego** (przycisk w aplikacji) i uruchom ją ponownie.
3. *Modele Whisper* → pobierz model (domyślnie *Base*; dla polskiego najlepszy *Large v3 Turbo*).
4. *Nowy wykład* → wybierz okno Teams, przeciągnij prostokąt na podglądzie wokół samej prezentacji, *Test dźwięku*, *Rozpocznij nagrywanie*.

Podczas nagrywania: *Pauza/Wznów*, *Zapisz slajd* (lub globalnie **⌘⇧S / Ctrl+Shift+S**), *Zatrzymaj i zapisz*.
Wskaźnik nagrywania: czerwona kropka w aplikacji, tytuł okna oraz licznik przy ikonie w pasku menu.

## Architektura

```
crates/
  lc-core      platformowo niezależny rdzeń (100% testowalny bez uprawnień)
    detect/      analiza klatek, porównanie odporne na przesunięcia, bloki, komponenty, dHash, maszyna stanów
    audio/       resampler sinc, mikser z zegarem, miernik, VAD + chunking z nakładką, WAV, Ogg/Opus
    transcript   słowa z czasami, scalanie chunków, przypisanie do slajdów
    session/     układ folderu, manifest v1, journal, zapisy atomowe, odzyskiwanie, edycje
    export/      PNG/WebP lossless, lecture.md, full.md, by-slide.md, PROMPT.md, ZIP
    pipeline/    Recorder (wątki: obraz → detektor → zapis; audio → Opus + VAD → kolejka; transkrypcja)
  lc-capture   ScreenCaptureKit (macOS), Windows Graphics Capture + WASAPI (Windows), mikrofon (cpal)
  lc-whisper   whisper.cpp (whisper-rs, Metal) + menedżer modeli
  lc-cli       `lecturecapture-cli` – nagrywanie i diagnostyka bez GUI
src-tauri/     aplikacja Tauri 2 (komendy, zasobnik, blokada uśpienia, skrót globalny, odzyskiwanie przy starcie)
src/           interfejs React + TypeScript + Tailwind
```

Implementacje platformowe są wymienne – rekorder zna tylko traity `VideoSource`, `AudioSource`, `Transcriber`
(`crates/lc-core/src/pipeline/traits.rs`).

### Wykrywanie slajdów
1. Próbkowanie 0,5–4 kl./s (domyślnie 2). Na macOS ScreenCaptureKit dostarcza klatkę tylko, gdy obraz się zmienił
   (`SCFrameStatus::Idle` przy statycznym slajdzie), więc statyczny slajd prawie nie kosztuje CPU.
2. Analizowany jest tylko zaznaczony obszar, zmniejszony do 320 px szerokości (skala szarości) – zapis zawsze w natywnej rozdzielczości.
3. Różnica pikseli z tolerancją przesunięcia ±1 px (szum kompresji Teams, drżenie skalowania) → siatka bloków →
   spójne komponenty. Zmiany mieszczące się w 2×2 blokach (kursor, drobne kontrolki) są ignorowane; maski użytkownika wykluczają obszary.
4. Zmiana musi być stabilna przez `stable_frames` próbek (debounce); niestabilna treść (wideo) zapisywana po `max_unstable_ms`.
5. Nowy obraz porównywany z galerią (dHash + porównanie pikselowe) → powrót do slajdu = nowe wystąpienie na osi czasu, ten sam plik.
6. Zmiana czysto addytywna na tle = etap stopniowego ujawniania → *merge* (slajd aktualizowany do pełnej wersji) lub *separate*.

Wszystkie progi: *Ustawienia → Wykrywanie slajdów* (pełna lista w `DetectorConfig`).

### Dźwięk i transkrypcja
* macOS: osobny strumień ScreenCaptureKit z dźwiękiem systemowym (wszystkie aplikacje lub tylko Teams),
  `excludesCurrentProcessAudio` chroni przed pętlą; działa niezależnie od urządzenia wyjściowego (słuchawki).
* Windows: WASAPI loopback wybranego wyjścia (cpal). Mikrofon opcjonalnie, z możliwością wyciszenia w trakcie.
* Wszystko → 16 kHz mono; mikser umieszcza próbki na osi czasu wykładu (zegar ścienny – poprawnie także po uśpieniu).
* Opus zapisywany strumieniowo (strona Ogg co 1 s, fsync co kilka s). VAD tnie mowę na fragmenty ≤ 28 s z 1 s nakładki;
  fragmenty trafiają do kolejki na dysku (`.lc/pending/`), więc transkrypcja nigdy nie blokuje nagrywania i nie trzyma nagrania w RAM.
* Wątek Whisper działa z obniżonym priorytetem (QoS utility); jeśli nie nadąża, kolejka rośnie i jest dokańczana po wykładzie.
* Znane halucynacje Whispera na ciszy („Napisy stworzone przez społeczność Amara.org”…) są odfiltrowywane.

### Niezawodność
* Manifest zapisywany atomowo po każdym zdarzeniu + heartbeat co 15 s; journal `.lc/events.jsonl`.
* Po awarii/zamknięciu: przy następnym starcie wykład jest automatycznie odzyskiwany (zamknięta oś czasu,
  sieroce PNG dołączone, uszkodzony koniec Ogg pominięty), a niedokończona transkrypcja wznawiana.
* Obsługa: utrata okna (zamknięte spotkanie) → automatyczne wznowienie, gdy okno wróci; utrata/zmiana urządzenia audio;
  uśpienie (luka wypełniana ciszą, oś czasu zachowana); brak sygnału → ostrzeżenie i wpis w „Braki danych”; mało miejsca na dysku.

## Testy

```bash
cargo test -p lc-core          # 52 testy: jednostkowe + integracyjne pipeline'u (syntetyczne źródła)
cargo test -p lc-whisper       # + prawdziwa transkrypcja (wymaga pobranego modelu `base`, inaczej SKIPPED)
npm run build                  # typecheck + build UI
```

Weryfikacja na urządzeniu (prawdziwe ScreenCaptureKit + dźwięk systemowy + Whisper, z uprawnieniami aplikacji):

```bash
open -n target/release/bundle/macos/LectureCapture.app --args --selftest /tmp/lc-selftest 45 base
cat /tmp/lc-selftest/selftest-report.json
```

CLI (wymaga uprawnienia „Nagrywanie ekranu” dla terminala):
`cargo run -p lc-cli -- sources | snapshot | record --window <id> --seconds 60 | download base | retranscribe <folder>`.

## Znane ograniczenia
* Jakość transkrypcji zależy od modelu: *Base* myli się w polskiej odmianie i łączy słowa; dla wykładów polecany *Large v3 Turbo*.
* Zmiana rozmiaru okna w trakcie nagrywania jest obsługiwana (przechwytywanie dostosowuje się do natywnej rozdzielczości),
  ale zaznaczony obszar jest względny – przy dużej zmianie proporcji okna zaznacz obszar ponownie.
* Prezentacja z ciągłą animacją/wideo zapisywana jest co `max_unstable_ms` (domyślnie 15 s).
* Echo: przy włączonym mikrofonie bez słuchawek głos prowadzącego trafi do nagrania dwukrotnie.
* Zamknięcie klapy MacBooka usypia system – nagrywanie wznowi się po wybudzeniu, a przerwa zostanie oznaczona.
* Windows: nie testowano na fizycznym sprzęcie; ramka przechwytywania WGC może być widoczna na Windows 10.
* Ochrona DRM / polityki organizacji: aplikacja ich nie omija – jeśli Teams blokuje przechwytywanie, obraz będzie czarny.

## Prywatność i zasady
Nagrywanie wykładów może wymagać zgody prowadzącego i musi być zgodne z regulaminem uczelni oraz polityką organizacji.
Materiały służą do osobistej nauki. Aplikacja nie wysyła żadnych danych.

Licencja: MIT. Komponenty zewnętrzne: [THIRD_PARTY.md](THIRD_PARTY.md).

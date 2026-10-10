# Instalacja LectureCapture

Najnowsza wersja: **[Releases → Latest](https://github.com/kokosx/LectureCapture/releases/latest)**

| System | Bezpośredni link |
|---|---|
| macOS 15 Sequoia lub nowszy, Apple Silicon (M1–M4) | [LectureCapture-macOS-AppleSilicon.dmg](https://github.com/kokosx/LectureCapture/releases/latest/download/LectureCapture-macOS-AppleSilicon.dmg) |
| Windows 10 / 11, 64‑bit | [LectureCapture-Windows-Setup.exe](https://github.com/kokosx/LectureCapture/releases/latest/download/LectureCapture-Windows-Setup.exe) |

> Aplikacja jest darmowa i nie ma certyfikatu Apple Developer ani Microsoft Authenticode, dlatego przy pierwszym
> uruchomieniu system pokaże ostrzeżenie. To normalne dla projektów open source – poniżej jak je ominąć.
> Kod źródłowy jest w tym repozytorium, a instalatory budują się automatycznie w GitHub Actions.

---

## macOS

### 1. Instalacja
1. Pobierz `LectureCapture-macOS-AppleSilicon.dmg`.
2. Otwórz plik DMG i przeciągnij **LectureCapture** do folderu **Aplikacje**.
3. Wysuń obraz DMG (ikona ⏏ w Finderze).

### 2. Pierwsze uruchomienie (aplikacja niepodpisana)
macOS zablokuje pierwsze otwarcie komunikatem *„Nie można otworzyć LectureCapture”* albo *„…jest uszkodzony”*.
Wybierz **jeden** sposób:

**A. Terminal (najprościej, jedno polecenie)**

```bash
xattr -dr com.apple.quarantine /Applications/LectureCapture.app
```

Potem uruchom aplikację normalnie z Launchpada / folderu Aplikacje.

**B. Ustawienia systemowe**
1. Spróbuj otworzyć aplikację – pojawi się ostrzeżenie, kliknij **Gotowe**.
2. Otwórz **Ustawienia systemowe → Prywatność i ochrona**, przewiń w dół.
3. Przy komunikacie o LectureCapture kliknij **Otwórz mimo to** i potwierdź hasłem / Touch ID.

### 3. Uprawnienia
1. Przy pierwszym starcie aplikacja poprosi o **Nagrywanie ekranu i dźwięku systemowego** – kliknij przycisk w aplikacji,
   włącz LectureCapture w *Ustawienia systemowe → Prywatność i ochrona → Nagrywanie ekranu i dźwięku systemowego*,
   a następnie **uruchom aplikację ponownie** (macOS tego wymaga).
2. Mikrofon (opcjonalnie) – system zapyta, gdy włączysz go w nowym wykładzie.
3. **Dostępność** (opcjonalnie) – tylko jeśli używasz *Zakończ o godzinie → Opuść spotkanie Teams*. Aplikacja wysyła wtedy
   skrót ⌘⇧H do okna spotkania, a macOS wymaga na to zgody: *Ustawienia systemowe → Prywatność i ochrona → Dostępność* →
   włącz LectureCapture (przycisk „Otwórz ustawienia” w aplikacji). Przy pierwszym użyciu system może też zapytać o
   sterowanie aplikacją „System Events” – kliknij **OK**.

> **Przełącznik jest włączony, a aplikacja dalej prosi o uprawnienie?** Wersje do 0.2.0 miały podpis powiązany
> z konkretnym plikiem, więc po każdej aktualizacji macOS traktował je jak nową aplikację. Kliknij w aplikacji
> **Napraw uprawnienia** (czyści stare wpisy), włącz LectureCapture w oknie systemowym i **Uruchom ponownie**.
> To samo z Terminala: `tccutil reset ScreenCapture app.lecturecapture && tccutil reset Microphone app.lecturecapture`.
> Od kolejnej wersji podpis opiera się na identyfikatorze aplikacji, więc uprawnienia zostają po aktualizacjach.
>
> Do samej transkrypcji na sali (tryb **Na sali (tylko dźwięk)** w *Nowym wykładzie*) uprawnienie nagrywania
> ekranu nie jest potrzebne – wystarczy mikrofon.

### Mac z procesorem Intel
Gotowy instalator jest tylko dla Apple Silicon. Na Macu z Intelem zbuduj aplikację ze źródeł (niżej).

---

## Windows

### 1. Instalacja
1. Pobierz `LectureCapture-Windows-Setup.exe`.
2. Uruchom plik. Jeśli pojawi się niebieskie okno **„System Windows ochronił ten komputer”** (SmartScreen):
   kliknij **Więcej informacji → Uruchom mimo to**.
3. Instalator nie wymaga uprawnień administratora (instaluje dla bieżącego użytkownika).
   Jeśli system nie ma środowiska **WebView2** (rzadkie na Windows 10/11), instalator pobierze je automatycznie.
4. Aplikację znajdziesz w menu Start jako **LectureCapture**.

### 2. Uprawnienia
Windows nie wymaga osobnej zgody na przechwytywanie ekranu. Dla mikrofonu sprawdź
*Ustawienia → Prywatność i zabezpieczenia → Mikrofon → Zezwalaj aplikacjom klasycznym na dostęp do mikrofonu*.

> Wersja Windows jest zbudowana i przetestowana automatycznie w CI, ale nie była jeszcze sprawdzana na fizycznym
> komputerze. Jeśli coś nie działa – zgłoś to w [Issues](https://github.com/kokosx/LectureCapture/issues).

### Odinstalowanie
*Ustawienia → Aplikacje → Zainstalowane aplikacje → LectureCapture → Odinstaluj.*

---

## Po instalacji (oba systemy)

1. Potwierdź informację o zasadach nagrywania (zgoda prowadzącego/uczelni).
2. **Modele Whisper** → pobierz model. Domyślnie *Base* (~150 MB, szybki); dla polskich wykładów najlepszy
   *Large v3 Turbo* (~1,6 GB). Model pobiera się raz i działa w pełni offline.
3. **Nowy wykład** → wybierz okno Teams → zaznacz prostokąt wokół prezentacji → *Test dźwięku* → *Rozpocznij nagrywanie*.
4. Po wykładzie otwórz folder wykładu i użyj `PROMPT.md` z Claude / ChatGPT (szczegóły w [README](README.md#jak-zrobić-materiały-do-nauki-z-claude)).

## Aktualizacja
Pobierz nowy instalator z [Releases](https://github.com/kokosx/LectureCapture/releases/latest) i zainstaluj na starą wersję
(macOS: zastąp aplikację w folderze Aplikacje i ponownie wykonaj krok z `xattr`). Nagrane wykłady i pobrane modele zostają.

## Odinstalowanie na macOS
Przenieś `LectureCapture.app` z Aplikacji do Kosza. Pobrane modele i ustawienia leżą w
`~/Library/Application Support/app.lecturecapture` – usuń ten folder, jeśli chcesz odzyskać miejsce.
Nagrane wykłady są w folderze wybranym w ustawieniach (domyślnie `~/Documents/LectureCapture`).

---

## Budowanie ze źródeł

### macOS
Wymagania: macOS 15+, Xcode Command Line Tools (`xcode-select --install`), [Rust](https://rustup.rs) (stable),
Node.js 20+, CMake (`brew install cmake`).

```bash
git clone https://github.com/kokosx/LectureCapture && cd LectureCapture
npm install
npx tauri build --bundles dmg    # → target/release/bundle/dmg/LectureCapture_*.dmg
```

### Windows
Wymagania: [Rust](https://rustup.rs) (toolchain MSVC), Node.js 20+, CMake,
Visual Studio Build Tools z komponentem *Programowanie aplikacji klasycznych w języku C++*.

```bash
git clone https://github.com/kokosx/LectureCapture && cd LectureCapture
npm install
npx tauri build --bundles nsis   # → target\release\bundle\nsis\LectureCapture_*-setup.exe
```

## Wydawanie nowej wersji (dla opiekuna repozytorium)
1. Podnieś wersję w `package.json`, `Cargo.toml` (workspace) i `src-tauri/tauri.conf.json`.
2. `git tag v0.2.0 && git push origin v0.2.0`
3. Workflow **Release** zbuduje DMG i EXE i opublikuje wydanie (ok. 20–30 min).

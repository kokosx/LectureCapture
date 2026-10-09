# Komponenty zewnętrzne i licencje

| Komponent | Zastosowanie | Licencja |
|---|---|---|
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (przez `whisper-rs` / `whisper-rs-sys`) | lokalna transkrypcja | MIT (whisper-rs: Unlicense) |
| Modele Whisper (`ggml-*.bin`, [ggerganov/whisper.cpp na Hugging Face](https://huggingface.co/ggerganov/whisper.cpp)) | wagi modeli – **nie są dołączane do instalatora**, pobierane na żądanie z weryfikacją SHA-256 | MIT (OpenAI Whisper) |
| [libopus](https://opus-codec.org) (przez `audiopus_sys`, linkowany statycznie) | kompresja nagrania | BSD-3-Clause |
| [screencapturekit-rs](https://github.com/doom-fish/screencapturekit-rs) | przechwytywanie obrazu i dźwięku na macOS | MIT / Apache-2.0 |
| [windows-capture](https://github.com/NiiightmareXD/windows-capture) | Windows Graphics Capture | MIT |
| [cpal](https://github.com/RustAudio/cpal) | mikrofon, WASAPI loopback | Apache-2.0 |
| [Tauri 2](https://tauri.app) + wtyczki | aplikacja desktopowa | MIT / Apache-2.0 |
| [png](https://github.com/image-rs/image-png), [image](https://github.com/image-rs/image) (WebP lossless) | zapis slajdów | MIT / Apache-2.0 |
| [ogg](https://github.com/RustAudio/ogg) | kontener Ogg | BSD-3-Clause |
| [zip](https://github.com/zip-rs/zip2) | eksport ZIP | MIT |
| React, Vite, Tailwind CSS, lucide-react | interfejs | MIT / ISC |

Pełne listy zależności: `cargo tree` oraz `npm ls`.

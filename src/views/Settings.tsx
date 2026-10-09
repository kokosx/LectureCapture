import { useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RotateCcw } from "lucide-react";
import { api, Settings, SystemInfo } from "../lib/api";
import { Button, Card, Field, Input, Select, Toggle, useToast } from "../components/ui";
import type { Nav } from "../App";

function Num({ value, onChange, min, max, step = 1, suffix }: {
  value: number; onChange: (v: number) => void; min?: number; max?: number; step?: number; suffix?: string;
}) {
  return (
    <div className="flex items-center gap-2">
      <Input type="number" value={value} min={min} max={max} step={step} onChange={(e) => onChange(Number(e.target.value))} className="w-28 tabular-nums" />
      {suffix && <span className="text-muted text-[12px]">{suffix}</span>}
    </div>
  );
}

export function SettingsView({ nav }: { nav: Nav }) {
  const toast = useToast();
  const [s, setS] = useState<Settings>(nav.settings);
  const [info, setInfo] = useState<SystemInfo | null>(null);
  useEffect(() => { api.systemInfo().then(setInfo); }, []);
  const dirty = JSON.stringify(s) !== JSON.stringify(nav.settings);
  const set = (patch: Partial<Settings>) => setS({ ...s, ...patch });
  const det = (patch: Partial<Settings["detector"]>) => setS({ ...s, detector: { ...s.detector, ...patch } });
  const aud = (patch: Partial<Settings["audio"]>) => setS({ ...s, audio: { ...s.audio, ...patch } });
  const tr = (patch: Partial<Settings["transcription"]>) => setS({ ...s, transcription: { ...s.transcription, ...patch } });

  return (
    <div className="max-w-[900px] mx-auto px-8 pb-24">
      <h1 className="text-[22px] font-semibold tracking-tight mb-5">Ustawienia</h1>
      <div className="space-y-4">
        <Card title="Ogólne">
          <div className="space-y-4">
            <Field label="Domyślny folder wykładów">
              <div className="flex gap-2">
                <Input value={s.lectures_root} onChange={(e) => set({ lectures_root: e.target.value })} />
                <Button icon={<FolderOpen size={14} />} onClick={async () => {
                  const d = await openDialog({ directory: true, defaultPath: s.lectures_root });
                  if (typeof d === "string") set({ lectures_root: d });
                }} />
              </div>
            </Field>
            <div className="grid grid-cols-2 gap-4">
              <Field label="Motyw">
                <Select value={s.theme} onChange={(v) => set({ theme: v })} options={[{ value: "system", label: "Systemowy" }, { value: "light", label: "Jasny" }, { value: "dark", label: "Ciemny" }]} />
              </Field>
              <Field label="Skrót „Zapisz slajd” (globalny, tylko podczas nagrywania)">
                <Input value={s.capture_shortcut} onChange={(e) => set({ capture_shortcut: e.target.value })} />
              </Field>
            </div>
            <Toggle checked={s.keep_awake} onChange={(v) => set({ keep_awake: v })} label="Nie usypiaj komputera podczas nagrywania" description="Blokuje automatyczne uśpienie systemu na czas wykładu (zamknięcie klapy nadal usypia)." />
          </div>
        </Card>

        <Card title="Wykrywanie slajdów">
          <div className="grid grid-cols-2 gap-4">
            <Field label="Częstotliwość próbkowania" hint="1–2 kl./s wystarcza; mniej = mniejsze zużycie baterii.">
              <Num value={s.detector.sample_fps} step={0.5} min={0.5} max={4} onChange={(v) => det({ sample_fps: v })} suffix="kl./s" />
            </Field>
            <Field label="Stabilność (kolejne identyczne próbki)" hint="Debounce – slajd zapisywany dopiero po ustabilizowaniu.">
              <Num value={s.detector.stable_frames} min={1} max={10} onChange={(v) => det({ stable_frames: v })} />
            </Field>
            <Field label="Próg różnicy piksela" hint="Wyżej = mniej wrażliwy na szum kompresji wideo Teams.">
              <Num value={s.detector.pixel_threshold} min={5} max={120} onChange={(v) => det({ pixel_threshold: v })} suffix="/ 255" />
            </Field>
            <Field label="Ignoruj zmiany mniejsze niż (bloki)" hint="Kursor i drobne elementy UI mieszczą się w małym obszarze.">
              <Num value={s.detector.cursor_max_blocks} min={1} max={8} onChange={(v) => det({ cursor_max_blocks: v })} suffix="× blok" />
            </Field>
            <Field label="Zapis niestabilnej treści po" hint="Np. wideo lub animacja na slajdzie.">
              <Num value={s.detector.max_unstable_ms / 1000} min={3} max={120} onChange={(v) => det({ max_unstable_ms: v * 1000 })} suffix="s" />
            </Field>
            <Field label="Stopniowe ujawnianie punktów">
              <Select value={s.detector.reveal_mode} onChange={(v) => det({ reveal_mode: v })} options={[
                { value: "merge", label: "Jeden slajd – zachowaj pełną wersję" },
                { value: "separate", label: "Każdy etap jako osobny slajd" },
              ]} />
            </Field>
          </div>
          <div className="mt-4">
            <Toggle checked={s.detector.ignore_blank} onChange={(v) => det({ ignore_blank: v })} label="Ignoruj puste (czarne/jednolite) klatki" description="Np. gdy prowadzący przestaje udostępniać ekran." />
          </div>
        </Card>

        <Card title="Dźwięk">
          <div className="grid grid-cols-2 gap-4">
            <Field label="Jakość Opus" hint="Mowa: 24–32 kb/s w zupełności wystarcza.">
              <Select value={String(s.audio.opus_bitrate)} onChange={(v) => aud({ opus_bitrate: Number(v) })} options={[
                { value: "16000", label: "16 kb/s (~7 MB/h)" }, { value: "24000", label: "24 kb/s (~11 MB/h)" },
                { value: "32000", label: "32 kb/s (~14 MB/h)" }, { value: "48000", label: "48 kb/s (~22 MB/h)" },
              ]} />
            </Field>
            <Field label="Po transkrypcji">
              <Select value={s.audio.retention} onChange={(v) => aud({ retention: v })} options={[
                { value: "keep", label: "Zachowaj nagranie (Opus)" },
                { value: "delete_after_transcription", label: "Usuń audio po udanej transkrypcji" },
              ]} />
            </Field>
            <Field label="Ostrzeż o braku sygnału po">
              <Num value={s.audio.silence_warn_ms / 1000} min={5} max={600} onChange={(v) => aud({ silence_warn_ms: v * 1000 })} suffix="s" />
            </Field>
            <Field label="Wzmocnienie mikrofonu">
              <Num value={s.audio.microphone_gain} step={0.1} min={0} max={4} onChange={(v) => aud({ microphone_gain: v })} suffix="×" />
            </Field>
          </div>
        </Card>

        <Card title="Transkrypcja">
          <div className="grid grid-cols-2 gap-4">
            <Field label="Wątki CPU dla Whisper" hint="Mniej = lżej dla systemu. GPU (Metal) jest używane automatycznie.">
              <Num value={s.transcription.threads} min={1} max={16} onChange={(v) => tr({ threads: v })} />
            </Field>
            <Field label="Beam search" hint="1 = najszybciej; 5 = dokładniej, wolniej.">
              <Num value={s.transcription.beam_size} min={1} max={8} onChange={(v) => tr({ beam_size: v })} />
            </Field>
          </div>
          <div className="mt-4">
            <Field label="Słownik / kontekst (opcjonalnie)" hint="Np. nazwy własne i terminy z przedmiotu – pomagają w rozpoznawaniu. Krótko, po przecinku.">
              <Input value={s.transcription.initial_prompt ?? ""} onChange={(e) => tr({ initial_prompt: e.target.value || null })} placeholder="np. drzewo AVL, kopiec, Dijkstra, złożoność O(n log n)" />
            </Field>
          </div>
        </Card>

        <Card title="Pliki">
          <div className="grid grid-cols-2 gap-4 items-start">
            <Field label="Kompresja PNG (zawsze bezstratna)">
              <Select value={s.output.png_compression} onChange={(v) => setS({ ...s, output: { ...s.output, png_compression: v } })} options={[
                { value: "fast", label: "Szybka" }, { value: "balanced", label: "Zrównoważona" }, { value: "best", label: "Najmniejsze pliki" },
              ]} />
            </Field>
            <div className="pt-5">
              <Toggle checked={s.output.webp_archive} onChange={(v) => setS({ ...s, output: { ...s.output, webp_archive: v } })} label="Kopia archiwalna WebP (lossless)" description="Dodatkowo w slides/archive/. PNG pozostaje formatem podstawowym." />
            </div>
          </div>
        </Card>

        {info && (
          <Card title="System">
            <dl className="grid grid-cols-[150px_1fr] gap-y-1 text-[12.5px] selectable">
              <dt className="text-muted">Wersja</dt><dd>LectureCapture {info.version} · {info.platform}/{info.arch}</dd>
              <dt className="text-muted">Modele</dt><dd className="break-all">{info.models_dir}</dd>
              <dt className="text-muted">whisper.cpp</dt><dd className="break-all text-[11.5px] text-muted">{info.whisper}</dd>
            </dl>
          </Card>
        )}
      </div>

      <div className="fixed bottom-0 left-[214px] right-0 border-t border-line bg-panel/90 backdrop-blur px-8 py-3 flex justify-end gap-2">
        <Button variant="ghost" icon={<RotateCcw size={13} />} disabled={!dirty} onClick={() => setS(nav.settings)}>Cofnij zmiany</Button>
        <Button variant="primary" disabled={!dirty} onClick={async () => {
          try {
            await nav.updateSettings(s);
            toast("Ustawienia zapisane", "success");
          } catch (e) {
            toast(String(e), "error");
          }
        }}>Zapisz</Button>
      </div>
    </div>
  );
}

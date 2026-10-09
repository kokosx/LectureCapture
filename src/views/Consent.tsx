import { useEffect, useState } from "react";
import { ShieldCheck, MonitorPlay, Lock } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, Permissions } from "../lib/api";
import { Button, Badge } from "../components/ui";
import type { Nav } from "../App";

export const PRIVACY_URL_MAC = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

export function PermissionRow({ compact }: { compact?: boolean }) {
  const [perm, setPerm] = useState<Permissions | null>(null);
  const refresh = () => api.permissions().then(setPerm);
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 2000);
    return () => clearInterval(t);
  }, []);
  if (!perm) return null;
  if (perm.screen) {
    return compact ? null : (
      <div className="flex items-center gap-2 text-[12.5px]"><Badge tone="green">✓</Badge> Uprawnienie do nagrywania ekranu i dźwięku jest nadane.</div>
    );
  }
  return (
    <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-[12.5px]">
      <div className="font-medium mb-1">Brak uprawnienia „Nagrywanie ekranu i dźwięku systemowego”</div>
      <div className="text-muted mb-2.5">
        macOS wymaga Twojej zgody, aby aplikacja mogła zapisywać slajdy i dźwięk z Teams. Po nadaniu uprawnienia uruchom
        LectureCapture ponownie.
      </div>
      <div className="flex gap-2">
        <Button size="sm" variant="primary" onClick={() => api.requestScreenPermission().then(refresh)}>Poproś o uprawnienie</Button>
        <Button size="sm" onClick={() => openUrl(PRIVACY_URL_MAC)}>Otwórz Ustawienia systemowe</Button>
      </div>
    </div>
  );
}

export function Consent({ nav }: { nav: Nav }) {
  const [checked, setChecked] = useState(false);
  return (
    <div className="fixed inset-0 z-50 bg-black/45 backdrop-blur-[3px] flex items-center justify-center p-6">
      <div className="bg-panel border border-line rounded-2xl shadow-2xl w-[620px] max-h-full overflow-auto">
        <div className="p-7">
          <div className="flex items-center gap-3 mb-4">
            <img src="/icon.png" className="w-11 h-11 rounded-xl" alt="" />
            <div>
              <h1 className="text-[18px] font-semibold">Witaj w LectureCapture</h1>
              <div className="text-muted">Lokalny rejestrator wykładów z Microsoft Teams</div>
            </div>
          </div>
          <div className="space-y-3.5 text-[13px] leading-relaxed">
            <div className="flex gap-3">
              <MonitorPlay size={18} className="text-accent shrink-0 mt-0.5" />
              <div>
                Aplikacja zapisuje slajdy z wybranego okna lub obszaru ekranu, nagrywa dźwięk wykładu i tworzy transkrypcję
                <b> wyłącznie na tym komputerze</b>. Nagrywanie startuje dopiero po kliknięciu „Rozpocznij” i jest zawsze
                wyraźnie sygnalizowane.
              </div>
            </div>
            <div className="flex gap-3">
              <Lock size={18} className="text-accent shrink-0 mt-0.5" />
              <div>
                Brak kont, kluczy API, telemetrii i wysyłania danych. Jedyne połączenie sieciowe to pobranie wybranego modelu
                Whisper z Hugging Face – tylko gdy sam o to poprosisz.
              </div>
            </div>
            <div className="flex gap-3">
              <ShieldCheck size={18} className="text-amber-500 shrink-0 mt-0.5" />
              <div>
                <b>Odpowiedzialność za nagrywanie.</b> Rejestrowanie wypowiedzi może wymagać zgody prowadzącego i uczestników
                oraz musi być zgodne z regulaminem uczelni, polityką organizacji w Microsoft Teams i prawem autorskim. Materiały
                służą do Twojej osobistej nauki – nie publikuj ich bez zgody. Aplikacja nie omija żadnych zabezpieczeń Teams.
              </div>
            </div>
          </div>
          <div className="mt-5"><PermissionRow /></div>
          <label className="mt-5 flex items-start gap-2.5 cursor-pointer">
            <input type="checkbox" className="mt-0.5 accent-[var(--color-accent)]" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
            <span>Rozumiem i będę nagrywać wykłady zgodnie z zasadami uczelni oraz po uzyskaniu wymaganych zgód.</span>
          </label>
        </div>
        <div className="px-7 py-4 border-t border-line flex justify-end">
          <Button
            variant="primary"
            disabled={!checked}
            onClick={() => nav.updateSettings({ ...nav.settings, consent_acknowledged: true })}
          >
            Rozumiem, przejdź dalej
          </Button>
        </div>
      </div>
    </div>
  );
}

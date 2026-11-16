import { useEffect, useState } from "react";
import { Download, RefreshCw, ShieldCheck, SlidersHorizontal } from "lucide-react";
import { DEMO, autostart } from "../api";
import { Card, Toggle } from "../components";
import type { Self, Settings } from "../types";

interface Props {
  self: Self | null;
  version: string;
  save: (s: Settings) => void;
  checkUpdates: () => Promise<void>;
}

export default function SettingsView({ self, version, save, checkUpdates }: Props) {
  const [name, setName] = useState("");
  const [startup, setStartup] = useState<boolean | null>(null);
  const [checking, setChecking] = useState(false);
  const settings = self?.settings;

  useEffect(() => {
    if (self) setName(self.settings.name ?? self.info.name);
  }, [self]);
  useEffect(() => {
    autostart.get().then(setStartup).catch(() => setStartup(null));
  }, []);

  if (!self || !settings) return null;

  const saveName = () => {
    const next = name.trim();
    if (next && next !== self.info.name) save({ ...settings, name: next });
  };
  const toggleStartup = (on: boolean) => {
    setStartup(on);
    autostart.set(on).catch(() => setStartup(!on));
  };

  return (
    <div className="grid g2">
      <Card title="This computer" sub="How your other devices see it" icon={<SlidersHorizontal size={16} />}>
        <div className="field">
          <label htmlFor="name">Name</label>
          <input id="name" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} onBlur={saveName} onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
        </div>
        {startup !== null && (
          <Toggle on={startup} onChange={toggleStartup} label="Open OmniDesk when I sign in" hint="Keeps this computer reachable from your other devices. It starts in the tray." />
        )}
      </Card>

      <Card title="Sharing" sub="What paired devices may do with this computer" icon={<ShieldCheck size={16} />}>
        <Toggle on={settings.allow_view} onChange={(on) => save({ ...settings, allow_view: on })} label="Show this screen" hint="Paired devices can watch this screen live." />
        <Toggle on={settings.allow_control} onChange={(on) => save({ ...settings, allow_control: on })} label="Allow control" hint="Paired devices can use this computer's keyboard and mouse." />
        <Toggle on={settings.share_clipboard} onChange={(on) => save({ ...settings, share_clipboard: on })} label="Share copied text" hint="Copy on one device, paste on another." />
      </Card>

      <Card title="Updates" sub={DEMO ? "Browser preview" : `OmniDesk ${version}`} icon={<Download size={16} />}>
        <p className="muted note">OmniDesk checks for a new version when it opens and every few hours.</p>
        <button
          className="btn"
          disabled={checking || DEMO}
          onClick={async () => {
            setChecking(true);
            await checkUpdates();
            setChecking(false);
          }}
        >
          <RefreshCw size={14} className={checking ? "spin" : ""} />Check for updates
        </button>
      </Card>
    </div>
  );
}

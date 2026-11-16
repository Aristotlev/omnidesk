import { useCallback, useEffect, useRef, useState } from "react";
import { Download, LayoutGrid, MonitorSmartphone, Settings as SettingsIcon, Smartphone } from "lucide-react";
import "./styles.css";
import { api, checkForUpdate, DEMO, forgetStreams, onDevices, onEvent, type PendingUpdate } from "./api";
import type { Device, Self, Settings } from "./types";
import Wall from "./views/Wall";
import Devices, { type Offer } from "./views/Devices";
import SettingsView from "./views/SettingsView";
import Control from "./views/Control";
import MacPermissions from "./views/MacPermissions";

type View = "wall" | "devices" | "settings";

const NAV: { id: View; label: string; icon: typeof LayoutGrid; title: string; sub: string }[] = [
  { id: "wall", label: "Screens", icon: LayoutGrid, title: "Screens", sub: "Every paired device, live. Click a screen to control it." },
  { id: "devices", label: "Devices", icon: Smartphone, title: "Devices", sub: "Pair, check and remove your devices" },
  { id: "settings", label: "Settings", icon: SettingsIcon, title: "Settings", sub: "Name, sharing and updates" },
];

const SIX_HOURS = 6 * 3600 * 1000;

export default function App() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [self, setSelf] = useState<Self | null>(null);
  const [view, setView] = useState<View>("wall");
  const [open, setOpen] = useState<{ device: string; screen: number } | null>(null);
  const [offer, setOffer] = useState<Offer | null>(null);
  const [streamErrors, setStreamErrors] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  const [update, setUpdate] = useState<PendingUpdate | null>(null);
  const [updatePct, setUpdatePct] = useState<number | null>(null);
  const toastTimer = useRef<number | undefined>(undefined);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 4000);
  }, []);

  const refreshSelf = useCallback(() => void api.self().then(setSelf).catch((e) => setError(String(e))), []);

  useEffect(() => {
    api.devices().then(setDevices).catch((e) => setError(String(e)));
    refreshSelf();
    api.appVersion().then(setVersion);

    const silentCheck = () => checkForUpdate().then(setUpdate).catch(() => {});
    silentCheck();
    const timer = window.setInterval(silentCheck, SIX_HOURS);

    const offs = [
      onDevices((list) => {
        // A device that went offline forgets its streams; ask again when it is back.
        setDevices((before) => {
          before.filter((d) => d.online && !list.some((n) => n.id === d.id && n.online)).forEach((d) => forgetStreams(d.id));
          return list;
        });
        refreshSelf();
        // A stream that failed gets a fresh start when its device comes back.
        setStreamErrors((errs) => Object.fromEntries(Object.entries(errs).filter(([key]) => list.some((d) => d.online && key.startsWith(d.id)))));
      }),
      onEvent<{ device: string; screen: number; reason: string }>("stream-error", (e) => setStreamErrors((errs) => ({ ...errs, [`${e.device}-${e.screen}`]: e.reason }))),
      onEvent<{ device: string; name: string }>("paired", (e) => {
        setOffer(null);
        showToast(`Paired with ${e.name}.`);
      }),
      onEvent<{ reason: string; code_valid: boolean }>("pairing-ended", (e) => {
        if (!e.code_valid) setOffer(null);
        showToast(e.reason);
      }),
      onEvent<string>("notice", setNotice),
    ];
    return () => {
      offs.forEach((off) => off());
      window.clearInterval(timer);
    };
  }, [refreshSelf, showToast]);

  const saveSettings = useCallback(
    (settings: Settings) => {
      setSelf((s) => (s ? { ...s, settings } : s));
      void api.setSettings(settings).then(refreshSelf).catch((e) => setError(String(e)));
    },
    [refreshSelf],
  );

  const checkUpdates = useCallback(async () => {
    try {
      const u = await checkForUpdate();
      setUpdate(u);
      if (!u) showToast("OmniDesk is up to date.");
    } catch (e) {
      setError(`Update check failed: ${e}`);
    }
  }, [showToast]);

  const installUpdate = async () => {
    if (!update) return;
    setUpdatePct(0);
    try {
      await update.install(setUpdatePct);
    } catch (e) {
      setUpdatePct(null);
      setError(`Update failed: ${e}`);
    }
  };

  const nav = NAV.find((n) => n.id === view)!;
  const opened = open ? devices.find((d) => d.id === open.device && d.paired) : undefined;
  const online = devices.filter((d) => d.paired && d.online).length;

  if (open && opened) {
    return (
      <>
        <Control
          device={opened}
          screen={open.screen}
          error={streamErrors[`${open.device}-${open.screen}`]}
          onScreen={(screen) => setOpen({ device: open.device, screen })}
          onClose={() => setOpen(null)}
        />
        {toast && <div className="toast">{toast}</div>}
      </>
    );
  }

  return (
    <div className="app">
      <aside className="side">
        <div className="brand"><div className="brand-mark"><MonitorSmartphone size={17} /></div><span className="brand-text">OmniDesk</span></div>
        {NAV.map((n) => (
          <button key={n.id} className={`nav ${view === n.id ? "active" : ""}`} onClick={() => setView(n.id)}>
            <n.icon size={17} />{n.label}
            {n.id === "wall" && online > 0 && <span className="count ok">{online}</span>}
          </button>
        ))}
        <div className="side-foot">{DEMO ? "Sample devices (browser preview)" : `v${version}`}</div>
      </aside>

      <main className="main">
        <div className="head">
          <div>
            <h1>{nav.title}</h1>
            <p>{nav.sub}</p>
          </div>
        </div>

        <MacPermissions />
        {update && (
          <div className="banner update">
            <Download size={16} />
            {updatePct === null
              ? <>OmniDesk {update.version} is available.</>
              : <>Installing OmniDesk {update.version}… {updatePct}%. The app restarts by itself.</>}
            {updatePct === null && <button className="btn primary" style={{ marginLeft: "auto" }} onClick={installUpdate}>Update now</button>}
          </div>
        )}
        {error && <div className="banner err">{error}<button className="btn ghost" style={{ marginLeft: "auto" }} onClick={() => setError(null)}>Dismiss</button></div>}
        {notice && <div className="banner">{notice}<button className="btn ghost" style={{ marginLeft: "auto" }} onClick={() => setNotice(null)}>Dismiss</button></div>}

        {view === "wall" && <Wall devices={devices} errors={streamErrors} onOpen={(device, screen) => setOpen({ device, screen })} goDevices={() => setView("devices")} />}
        {view === "devices" && <Devices self={self} devices={devices} offer={offer} setOffer={setOffer} onPaired={(name) => { showToast(`Paired with ${name}.`); setView("wall"); }} />}
        {view === "settings" && <SettingsView self={self} version={version} save={saveSettings} checkUpdates={checkUpdates} />}
      </main>
      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

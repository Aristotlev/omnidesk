import { useEffect, useState } from "react";
import { MousePointer2, MonitorUp, RotateCw, ShieldAlert } from "lucide-react";
import { api } from "../api";
import type { PermissionKind, Permissions } from "../types";

/**
 * macOS only: what this Mac still has to allow before other devices can see and control it.
 * Hidden everywhere else, and once everything is allowed.
 */
export default function MacPermissions() {
  const [perms, setPerms] = useState<Permissions | null>(null);
  const [askedScreen, setAskedScreen] = useState(false);

  useEffect(() => {
    let alive = true;
    const load = () => api.permissions().then((p) => alive && setPerms(p)).catch(() => {});
    load();
    // The person switches these on in System Settings; notice it without a restart where macOS allows.
    const t = window.setInterval(load, 2500);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
  }, []);

  if (!perms || (perms.screen !== "missing" && perms.control !== "missing")) return null;

  const ask = (kind: PermissionKind) => {
    if (kind === "screen") setAskedScreen(true);
    void api.requestPermission(kind);
  };

  return (
    <div className="banner perms">
      <ShieldAlert size={18} />
      <div className="perms-text">
        <div className="strong">Your other devices can't {perms.screen === "missing" && perms.control === "missing" ? "see or control" : perms.screen === "missing" ? "see" : "control"} this Mac yet.</div>
        <div className="muted small">
          {askedScreen && perms.screen === "missing"
            ? "In System Settings, turn on OmniDesk under Screen Recording, then click Reopen OmniDesk."
            : "macOS asks once. Click a button, turn on OmniDesk in the list that opens, and come back."}
        </div>
      </div>
      <div className="perms-actions">
        {perms.screen === "missing" &&
          (askedScreen ? (
            <button className="btn primary" onClick={() => void api.relaunch()}><RotateCw size={14} />Reopen OmniDesk</button>
          ) : (
            <button className="btn primary" onClick={() => ask("screen")}><MonitorUp size={14} />Allow Screen Recording</button>
          ))}
        {perms.control === "missing" && (
          <button className={`btn ${perms.screen === "missing" ? "" : "primary"}`} onClick={() => ask("control")}><MousePointer2 size={14} />Allow Accessibility</button>
        )}
      </div>
    </div>
  );
}

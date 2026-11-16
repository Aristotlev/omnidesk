import { MonitorOff, MousePointer2, Plus } from "lucide-react";
import { DeviceIcon, ScreenCanvas, describe } from "../components";
import type { Device } from "../types";

interface Props {
  devices: Device[];
  errors: Record<string, string>;
  onOpen: (device: string, screen: number) => void;
  goDevices: () => void;
}

function Placeholder({ d, reason }: { d: Device; reason: string }) {
  return (
    <div className="tile off">
      <div className="tile-view">
        <div className="tile-msg"><MonitorOff size={22} /><span>{reason}</span></div>
      </div>
      <div className="tile-foot">
        <DeviceIcon kind={d.kind} />
        <span className="strong ellipsis">{d.name}</span>
        <span className="muted small ellipsis">{describe(d)}</span>
      </div>
    </div>
  );
}

export default function Wall({ devices, errors, onOpen, goDevices }: Props) {
  const paired = devices.filter((d) => d.paired);

  if (paired.length === 0) {
    return (
      <div className="empty big">
        <h2>No screens yet</h2>
        <p>Install OmniDesk on your other computers, then pair them here. Their screens show up on this page, live.</p>
        <button className="btn primary" onClick={goDevices}><Plus size={15} />Add a device</button>
      </div>
    );
  }

  return (
    <div className="wall">
      {paired.map((d) => {
        if (!d.online) return <Placeholder key={d.id} d={d} reason="Offline. Open OmniDesk on this device." />;
        if (d.capabilities?.view !== "always" || d.screens.length === 0) {
          return <Placeholder key={d.id} d={d} reason={d.capabilities?.view === "on_demand" ? "Start sharing on the device to see it here." : "Screen sharing is turned off on this device."} />;
        }
        return d.screens.map((s) => {
          const error = errors[`${d.id}-${s.index}`];
          return (
            <button key={`${d.id}-${s.index}`} className="tile" onClick={() => onOpen(d.id, s.index)} title={d.capabilities?.control ? `Control ${d.name}` : `Watch ${d.name}`}>
              <div className="tile-view" style={{ aspectRatio: `${s.width} / ${s.height}` }}>
                {error ? <div className="tile-msg"><MonitorOff size={22} /><span>{error}</span></div> : <ScreenCanvas device={d.id} screen={s.index} mode="thumbnail" />}
                {!error && d.capabilities?.control && <span className="tile-hover"><MousePointer2 size={15} />Click to control</span>}
              </div>
              <div className="tile-foot">
                <DeviceIcon kind={d.kind} />
                <span className="strong ellipsis">{d.name}</span>
                {d.screens.length > 1 && <span className="muted small ellipsis">{s.name || `Display ${s.index + 1}`}</span>}
                <span className="pill ok">Live</span>
              </div>
            </button>
          );
        });
      })}
    </div>
  );
}

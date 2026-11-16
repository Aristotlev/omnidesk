import { useEffect, useState } from "react";
import { KeyRound, Link2, Plus, Radar, Trash2 } from "lucide-react";
import { api } from "../api";
import { Card, Code, DeviceIcon, Spinner, StatusPill, describe } from "../components";
import type { Device, Self } from "../types";

export interface Offer {
  code: string;
  until: number;
}

interface Props {
  self: Self | null;
  devices: Device[];
  offer: Offer | null;
  setOffer: (o: Offer | null) => void;
  onPaired: (name: string) => void;
}

function Countdown({ until, onDone }: { until: number; onDone: () => void }) {
  const [left, setLeft] = useState(Math.max(0, Math.round((until - Date.now()) / 1000)));
  useEffect(() => {
    const t = window.setInterval(() => {
      const s = Math.max(0, Math.round((until - Date.now()) / 1000));
      setLeft(s);
      if (s === 0) onDone();
    }, 500);
    return () => window.clearInterval(t);
  }, [until, onDone]);
  return <span className="num">{Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")}</span>;
}

/** Code box and button used to pair with one device (or one address). */
function PairForm({ target, label, onPaired, onCancel, address }: { target?: string; label: string; onPaired: (id: string) => void; onCancel: () => void; address?: boolean }) {
  const [where, setWhere] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const digits = code.replace(/\D/g, "");
  const ready = digits.length === 6 && (!address || where.trim().length > 0);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!ready || busy) return;
    setBusy(true);
    setError(null);
    try {
      onPaired(await api.pair(target ?? where.trim(), digits));
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <form className="pair-form" onSubmit={submit}>
      {address && <input value={where} onChange={(e) => setWhere(e.target.value)} placeholder="Address, for example 192.168.1.20" aria-label="Address of the other device" autoFocus />}
      <input className="code-input" value={code} onChange={(e) => setCode(e.target.value)} inputMode="numeric" maxLength={7} placeholder="6-digit code" aria-label={`Pairing code shown on ${label}`} autoFocus={!address} />
      <button className="btn primary" type="submit" disabled={!ready || busy}>{busy ? <Spinner /> : <Link2 size={14} />}Pair</button>
      <button className="btn ghost" type="button" onClick={onCancel}>Cancel</button>
      {error && <div className="form-err">{error}</div>}
    </form>
  );
}

export default function Devices({ self, devices, offer, setOffer, onPaired }: Props) {
  const [pairing, setPairing] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const paired = devices.filter((d) => d.paired);
  const nearby = devices.filter((d) => !d.paired);

  const showCode = async () => {
    const o = await api.beginPairing();
    setOffer({ code: o.code, until: Date.now() + o.seconds * 1000 });
  };
  const hideCode = () => {
    void api.cancelPairing();
    setOffer(null);
  };
  const done = (id: string) => {
    setPairing(null);
    onPaired(devices.find((d) => d.id === id)?.name ?? "the device");
  };

  return (
    <div className="grid">
      <Card
        title={self?.info.name ?? "This computer"}
        sub={self ? `This computer · ${describe(self.info)}${self.address ? ` · ${self.address}` : ""}` : undefined}
        icon={self ? <DeviceIcon kind={self.info.kind} /> : undefined}
        end={!offer && <button className="btn" onClick={showCode}><KeyRound size={14} />Show pairing code</button>}
      >
        {offer ? (
          <div className="offer">
            <Code code={offer.code} />
            <div>
              <div className="strong">Type this code on the device you are adding.</div>
              <div className="muted small">Valid for <Countdown until={offer.until} onDone={() => setOffer(null)} />. It works once.</div>
            </div>
            <button className="btn ghost" onClick={hideCode}>Cancel</button>
          </div>
        ) : (
          <p className="muted note">To pair two devices, show a pairing code on one and type it on the other. Paired devices can see each other's screens, control each other and share copied text. You can switch each of those off in Settings.</p>
        )}
      </Card>

      <Card title="Add a device" sub="Other devices running OmniDesk on this network" icon={<Radar size={16} />}>
        {nearby.length === 0 && pairing !== "address" && (
          <div className="empty">
            No new devices found. Open OmniDesk on the other device and keep both on the same Wi-Fi or cable network.
          </div>
        )}
        {nearby.map((d) => (
          <div key={d.id} className="row-item">
            <div className="icon-chip"><DeviceIcon kind={d.kind} /></div>
            <div className="grow">
              <div className="strong ellipsis">{d.name}</div>
              <div className="muted small">{describe(d)}{d.address ? ` · ${d.address}` : ""}</div>
            </div>
            {pairing === d.id ? (
              <PairForm target={d.id} label={d.name} onPaired={done} onCancel={() => setPairing(null)} />
            ) : (
              <button className="btn primary" onClick={() => setPairing(d.id)}><Plus size={14} />Pair</button>
            )}
          </div>
        ))}
        {pairing === "address" ? (
          <div className="row-item">
            <div className="icon-chip"><Link2 size={16} /></div>
            <PairForm address label="the other device" onPaired={done} onCancel={() => setPairing(null)} />
          </div>
        ) : (
          <button className="linkish add-address" onClick={() => setPairing("address")}>Device not listed? Add it by address</button>
        )}
      </Card>

      <Card title="Paired devices" sub={paired.length ? `${paired.filter((d) => d.online).length} of ${paired.length} online` : undefined} icon={<Link2 size={16} />}>
        {paired.length === 0 && <div className="empty">No paired devices yet.</div>}
        {paired.map((d) => (
          <div key={d.id} className="row-item">
            <div className="icon-chip"><DeviceIcon kind={d.kind} /></div>
            <div className="grow">
              <div className="strong ellipsis">{d.name}</div>
              <div className="muted small">{describe(d)}{d.address ? ` · ${d.address}` : ""}</div>
            </div>
            <StatusPill d={d} />
            {removing === d.id ? (
              <>
                <button className="btn danger" onClick={() => { void api.unpair(d.id); setRemoving(null); }}>Remove {d.name}</button>
                <button className="btn ghost" onClick={() => setRemoving(null)}>Keep</button>
              </>
            ) : (
              <button className="btn ghost danger" onClick={() => setRemoving(d.id)} aria-label={`Remove ${d.name}`}><Trash2 size={14} />Remove</button>
            )}
          </div>
        ))}
      </Card>
    </div>
  );
}

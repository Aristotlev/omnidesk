# OmniDesk

**Every screen you own on one desk.** See the screens of all your computers side by side, live. Click one and your keyboard and mouse work on it. Copy text on one computer and paste it on another.

<p>
  <a href="https://github.com/Aristotlev/omnidesk/releases/latest/download/OmniDesk-Windows.exe"><b>⬇ Download for Windows</b></a> &nbsp;·&nbsp;
  <a href="https://github.com/Aristotlev/omnidesk/releases/latest/download/OmniDesk-Mac.dmg"><b>⬇ Download for Mac</b></a> &nbsp;·&nbsp;
  <a href="https://github.com/Aristotlev/omnidesk/releases/latest/download/OmniDesk-Linux.AppImage"><b>⬇ Download for Linux</b></a>
</p>

Windows 10 / 11, macOS 11 or later (Apple silicon and Intel), and Linux. Free. Once installed, OmniDesk updates itself.

## What you get

- **All your screens, live.** Every paired computer shows up on one page, every monitor of it, updating as you watch.
- **Control any of them.** Click a screen to open it full size. Your mouse, keyboard and scroll wheel now work on that computer. Switch to *Watch only* any time.
- **One clipboard.** Copy text on your PC, paste it on your Mac.
- **Private by design.** Devices talk to each other directly on your own network, encrypted end to end. There are no accounts and no OmniDesk servers.
- **You decide.** Each computer has its own switches: show this screen, allow control, share copied text.

## Get started

1. Install OmniDesk on every computer you want on your desk.
2. On the computer you are adding, open **Devices** and click **Show pairing code**.
3. On the computer you are sitting at, open **Devices**, click **Pair** next to the other computer and type the 6-digit code.

That's it. Paired computers find each other again by themselves, also after a restart. OmniDesk keeps running in the tray (menu bar on a Mac) when you close its window, so your other computers can still reach it. In **Settings** you can have it open when you sign in.

Both computers must be on the same network (the same Wi-Fi or cable at home or in the office). If a computer doesn't show up, use **Add it by address** and type the address shown under its name on its own Devices page.

## First launch

OmniDesk is new and not yet signed with a paid Apple or Microsoft certificate, so your computer asks once before opening it:

- **Windows:** if you see "Windows protected your PC", click **More info**, then **Run anyway**. When Windows asks whether OmniDesk may use the network, click **Allow** (private networks).
- **Mac:** open the .dmg and drag OmniDesk to Applications. The first time you open it, macOS may say it can't verify the developer: go to **System Settings → Privacy & Security** and click **Open Anyway**. OmniDesk then shows two buttons, **Allow Screen Recording** (so your other computers can see this Mac) and **Allow Accessibility** (so they can control it). Click each, switch OmniDesk on in the list that opens, then click **Reopen OmniDesk**.
- **Linux:** make the AppImage executable (`chmod +x OmniDesk-Linux.AppImage`) and run it, or install the .deb. Screen sharing and control work on X11 sessions today. On Wayland, other computers can't see or control this one yet.

## iPhone and iPad

Apple does not let any app other than its own show or control an iPhone or iPad screen from another device. That's why no remote desktop app (TeamViewer, RustDesk, AnyDesk and the rest) can do it either. Apple's own *iPhone Mirroring* is also switched off in the EU.

What OmniDesk will do on iPhone and iPad (in development):

- **Use them as a screen and touchpad for your computers.** See and control your PC or Mac from your iPad or iPhone.
- **Show an iPhone or iPad screen on the wall.** You tap *Start sharing* on the device each time, which is the only way Apple allows. It can be watched, not controlled.

## Privacy

Everything stays on your devices. OmniDesk has no accounts, no servers and no analytics. Screens, input and copied text go straight from one of your devices to another over your own network, encrypted with keys that never leave the device. Only devices you paired with the 6-digit code can connect, and you can remove a device at any time on the Devices page. It is removed on both sides.

[Report a problem](https://github.com/Aristotlev/omnidesk/issues)

## FAQ

**Is it secure to let another computer control mine?**
Only computers you paired yourself, by typing a code shown on this screen, can connect. A code works once and runs out after two minutes. Every connection is encrypted and each computer proves its identity with its own key. You can switch off control on any computer in Settings.

**Does it work over the internet, away from home?**
Not yet. Today both devices need to be on the same network.

**How do updates work?**
OmniDesk checks for a new version when it opens and every few hours, and installs it in one click.

---

<details>
<summary><b>For developers</b></summary>

Built with Rust (core and networking), Tauri 2 (app shell) and React + TypeScript (interface). See [ARCHITECTURE.md](ARCHITECTURE.md) for how it works and what comes next.

```powershell
npm install
npm run tauri dev        # run the app
npm run dev              # browser preview with sample devices at http://localhost:1420
cargo test --workspace   # core tests: encryption, pairing, Flow, two-device end-to-end
cargo run -p omnidesk-node --bin omnideskd -- --help   # the node without a window
```

On Linux, install the build libraries first: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libclang-dev libxcb1-dev libxrandr-dev libdbus-1-dev libpipewire-0.3-dev libwayland-dev libegl-dev libgbm-dev libxdo-dev`.

**Two devices on one computer.** Run a second node in a terminal and pair the app with it:

```
cargo run -p omnidesk-node --bin omnideskd -- --data-dir ./second --port 47801 --name "Second"
code      # shows a pairing code for the app
```

**Release.** Double-click `Publish OmniDesk.cmd`. It bumps the version, tags and pushes; GitHub Actions (`ci/release.yml`, copied to `.github/workflows`) runs the tests, builds the Windows installer, the universal Mac .dmg and the Linux AppImage and .deb, publishes the release with `latest.json` for the updater, and attaches the fixed-name downloads `OmniDesk-Windows.exe`, `OmniDesk-Mac.dmg` and `OmniDesk-Linux.AppImage`. `release-keys\` holds the update signing key (git-ignored). Back it up: without it, installed copies can't verify new updates.

**Layout**

```
crates/
  omnidesk-proto       wire messages shared by every device
  omnidesk-link        device keys, paired list, Noise encryption, 6-digit pairing (SPAKE2)
  omnidesk-discovery   finding devices on the network (mDNS / Bonjour)
  omnidesk-platform    screens, capture, keyboard and mouse, clipboard per OS
  omnidesk-flow        one keyboard and mouse across every screen (edge crossing)
  omnidesk-node        a device on the network: connections, streams, input, clipboard; `omnideskd`
src-tauri/             app shell: window, tray, start at sign-in, updater, frame delivery
src/                   interface: Screens, full-size control, Devices, Settings
docs/                  website: https://aristotlev.github.io/omnidesk/
```

</details>

<h1 align="center">Q-Tiler</h1>

<p align="center">
  <b>English</b> · <a href="README.md">Русский</a>
</p>

<p align="center">
  <img src="assets/banner.svg" alt="Q-Tiler" width="100%">
</p>

<p align="center">
  <b>A tiling window manager for Windows with a neon Exposé and an infinite canvas</b>
</p>

<p align="center">
  <img alt="Windows" src="https://img.shields.io/badge/Windows-10%20%7C%2011-35e0ff?style=flat-square">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-2024-ff4fd8?style=flat-square">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-b6ff5c?style=flat-square">
</p>

<p align="center">
Tiles your windows into a grid, shows an Exposé with animated "snakes", a 3D window overview<br>
and an infinite canvas where your apps stay fully usable.
</p>

---

## ⬇️ Download

👉 Open the latest release on the **Releases** page and download the file from **Assets**.

| Your system | File to download |
|---|---|
| Windows 10 / 11 | `qtil.exe`: download and run, no installation needed |

- **Do not download** `Source code (zip)`: it is the source, not the program.
- **No Rust or terminal needed.** That is only for developers (see [Build](#-build-from-source)).
- If Windows shows "Windows protected your PC", click **More info → Run anyway**: the program is not code-signed.
- Q-Tiler runs in the background and listens for hotkeys. Quit: <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Q</kbd>.

---

## ✨ Features

| | What it does | Keys |
|---|---|---|
| 🧱 | **Tiling** up to 5 windows with smooth animation | <kbd>Alt</kbd>+<kbd>Q</kbd> |
| 🪞 | **Mirrored** layout | <kbd>Alt</kbd>+<kbd>E</kbd> |
| 🔁 | **Single-app tiling** | <kbd>Alt</kbd>+<kbd>R</kbd> |
| 🌀 | **Exposé**: cards in a ring and neon ribbons | <kbd>Alt</kbd>+<kbd>D</kbd> |
| 🧊 | **3D overview** of windows in a virtual room | <kbd>Alt</kbd>+<kbd>X</kbd> |
| 🗺️ | **Infinite canvas** | <kbd>Alt</kbd>+<kbd>C</kbd> |
| ⚙️ | **Settings**: themes, colors, animation length | <kbd>Alt</kbd>+<kbd>Z</kbd> |

### Tiling layouts

<p align="center">
  <img src="assets/tiling.svg" alt="Layouts for 2, 3, 4 and 5 windows" width="90%">
</p>

---

## 🗺️ Infinite canvas

Press <kbd>Alt</kbd>+<kbd>C</kbd> and all open windows are placed on one boundless field. Pan around, zoom from 25 to 200%, and arrange windows any way you like.

<p align="center">
  <img src="assets/canvas.svg" alt="Infinite canvas with windows" width="90%">
  <br>
  <sub>Windows on the canvas. The highlighted one is "live": it behaves like a normal app.</sub>
</p>

**A short click on a window** brings the real application out to the spot of its preview, at the current zoom. From there everything works as usual: mouse, keyboard, menus, dragging, text selection. <kbd>Alt</kbd>+<kbd>V</kbd> sends the window back to the canvas.

| Action | How |
|---|---|
| Pan the space | <kbd>Alt</kbd> + right mouse button and drag |
| Zoom | <kbd>Alt</kbd> + wheel, or <kbd>Alt</kbd>+<kbd>=</kbd> / <kbd>Alt</kbd>+<kbd>-</kbd> |
| Move a window | Left mouse button and drag |
| Use a window | Short click on it |
| Return a window to the canvas | <kbd>Alt</kbd>+<kbd>V</kbd> |
| Close the canvas | <kbd>Alt</kbd>+<kbd>C</kbd> |

> While the canvas is open, the other tiling hotkeys are disabled, except <kbd>Alt</kbd>+<kbd>C</kbd> and <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Q</kbd>.

---

## ⌨️ All hotkeys

<details>
<summary><b>Show the full list</b></summary>

<br>

| Keys | Action |
|---|---|
| <kbd>Alt</kbd>+<kbd>Q</kbd> | Tile / untile (3 most recent windows) |
| <kbd>Alt</kbd>+<kbd>A</kbd> / <kbd>S</kbd> | Add / remove a window (up to 5) |
| <kbd>Alt</kbd>+<kbd>W</kbd> | Main window (hold Alt, press W) |
| <kbd>Alt</kbd>+<kbd>E</kbd> | Mirror layout |
| <kbd>Alt</kbd>+<kbd>R</kbd> | Single-app tiling |
| <kbd>Alt</kbd>+<kbd>F</kbd> | Close the last tiled window |
| <kbd>Alt</kbd>+<kbd>D</kbd> | Exposé |
| <kbd>Alt</kbd>+<kbd>X</kbd> | 3D overview |
| <kbd>Alt</kbd>+<kbd>C</kbd> | Infinite canvas |
| <kbd>Alt</kbd>+<kbd>V</kbd> | Canvas: return window to the canvas |
| <kbd>Alt</kbd>+<kbd>Z</kbd> | Settings and bind list |
| <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Q</kbd> | Quit |

</details>

---

## 🛠️ Build from source

You need Windows and Rust (edition 2024, recent stable).

```
cargo build --release
```

Output: `target/release/qtil.exe`.

---

## ⚠️ Limitations

- Windows only.
- Windows running as administrator cannot be moved (a Windows restriction).
- Minimized and maximized windows are not placed on the canvas.
- If a hotkey is taken by another program, Q-Tiler shows a notice.
- Early project: save your work before testing.

## 🛡️ Window safety on the canvas

Before moving windows off-screen, the canvas starts a separate safety process (`qtil.exe --holst-watchdog`). If the main program crashes, it puts the windows back where they were.

---

<p align="center">
  Licensed under <a href="LICENSE">MIT</a>
</p>

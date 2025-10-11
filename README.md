GPTTrans (Windows)

A lightweight Windows tray app in Rust that translates clipboard text via the OpenAI API when you press Alt+F3. It writes the translation back to the clipboard, shows a scrollable egui output window, and displays Windows toast notifications. Runs from the system tray with no console or taskbar icon when idle.

Purpose
- Instant clipboard translation with a global hotkey
- Minimal friction: tray-only background app, no console
- Local config file, no persistent secrets elsewhere

Key Features
- Global hotkey: Alt+F3 to translate clipboard text
- System tray menu: Settings and Quit
- Scrollable output window (egui) with Copy button
- Windows toast notifications for status/errors
- Async HTTP client (reqwest + rustls) and clipboard-win I/O

Requirements
- Windows 10/11
- Rust toolchain (stable MSVC)
- OpenAI API Key

Configuration
- Local file: place `config.json` next to the executable (same directory as `gpttrans.exe`). Example:
  {
    "openai_api_key": "sk-...",
    "openai_model": "gpt-4o-mini",
    "target_lang": "English"
  }
- Example file included: `config.example.json`. Copy it next to the built exe and rename to `config.json`.
- Environment variables (if set) override the file for that session:
  - `OPENAI_API_KEY`
  - `OPENAI_MODEL`
  - `TARGET_LANG`

Build & Run
1) Build in a shell (keep tray app closed while building):
   - cd to the repo folder
   - cargo build --release
2) Place `config.json` next to `target\release\gpttrans.exe` (or use Settings after launch).
3) Launch: `target\release\gpttrans.exe`

Usage
- Copy any text, press Alt+F3 to translate. The translated text replaces the clipboard and an egui output window opens (scrollable) to review/copy.
- Right-click the tray icon to open the menu:
  - Settings: edit and save config.json (API key, model, target language)
  - Quit: exit the app

Security & Privacy
- The API key is read from `config.json` (next to the exe) or environment variables and sent directly to OpenAI over HTTPS (rustls). No additional persistence is used.

Troubleshooting
- Hotkey doesn’t trigger: another program may already register Alt+F3. Close conflicting apps. A configurable hotkey is planned.
- Tray menu missing/empty: ensure you right-click the icon. If still empty, exit and relaunch; we’re refining tray menu behavior across shells.
- Toasts don’t appear: enable Windows notifications for apps.
- GPU/WGPU init errors on some systems: we can switch egui backend if needed.

Known Limitations / TODO
- Single-instance guard: not yet enforced. Planned via a system-wide named mutex so only one instance runs.
- Configurable hotkey: planned fields in config.json for modifiers + key (e.g., Alt+F9).
- Better tray icon & theming: replace the placeholder icon and support light/dark.

Security
- Your API key is only read from the environment and sent directly to the OpenAI API over HTTPS (rustls). No local persistence is implemented.

Troubleshooting
- If Alt+F3 doesn't trigger, another program may already register that hotkey. Try closing conflicting apps or we can add a configurable hotkey.
- If toasts don't appear, ensure Windows notifications are enabled for apps.

License
This project is licensed under the MIT License - see the LICENSE file for details.

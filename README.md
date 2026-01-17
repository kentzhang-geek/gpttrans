# Echo

**Echo** is a lightweight Windows tray application that allows you to translate text (and images!) instantly using OpenAI's GPT models or local Ollama models.

1. **Copy** text or an image to your clipboard.
2. Press **Alt+F3**.
3. A window appears with the translation.

![Echo Screenshot](screenshot.png)

## Features

- **Clipboard-based**: seamless workflow.
- **Vision Support**: Translates text inside images (screenshots, etc.).
- **Streaming**: Text appears instantly as it is generated.
- **Local AI**: Support for Ollama models (e.g. gemma2, llava).
- **Customizable**: Change hotkeys, target language, and models.
- **Modern UI**: Built with Rust and egui.pboard translation with a global hotkey
- **100% FREE option** with local AI (Ollama) - no API costs!
- Minimal friction: tray-only background app, no console
- Privacy-focused: use local models or OpenAI

## ✨ Key Features
- **Configurable global hotkey** (Alt+F3, Ctrl+Shift+T, etc.)
- **Multiple AI backends**: OpenAI API, Ollama (free local AI), or any OpenAI-compatible API
- **Streaming translations**: real-time text updates for faster perceived speed
- **Modern frameless UI** with dark theme
- System tray menu: Settings and Quit
- Windows toast notifications for status/errors
- Async HTTP client (reqwest + rustls) and clipboard-win I/O

## 📋 Requirements
- Windows 10/11
- Rust toolchain (stable MSVC) - for building from source
- **Option A**: OpenAI API Key (paid)
- **Option B**: Ollama (FREE, local, private) - see setup below

## ⚙️ Configuration

### Using OpenAI API (Paid)
Create `config.json` next to the executable:
```json
  {
    "openai_api_key": "sk-...",
    "openai_model": "gpt-4o-mini",
  "target_lang": "English",
  "hotkey": "Alt+F3",
  "api_type": "openai",
  "api_base": "https://api.openai.com/v1"
}
```

### Using Ollama (FREE - Recommended!)
```json
{
  "openai_api_key": "",
  "openai_model": "gemma3:270m",
  "target_lang": "English",
  "hotkey": "Alt+F3",
  "api_type": "ollama",
  "api_base": "http://localhost:11434/v1"
}
```

### Configuration Options
- `openai_api_key`: Your API key (leave empty for Ollama)
- `openai_model`: Model name (e.g., `gpt-4o-mini`, `llama3.2:3b`)
- `target_lang`: Target language for translation
- `hotkey`: Global hotkey (e.g., `Alt+F3`, `Ctrl+Shift+T`, `Win+Q`)
- `api_type`: `openai`, `ollama`, or `openai-compatible`
- `api_base`: API endpoint URL

**Note**: Copy `config.example.json` to `config.json` and modify it. You can also edit settings through the UI (right-click tray icon → Settings).

## 🚀 Quick Start with FREE Local AI (Ollama)

### Step 1: Install Ollama
```powershell
# Option A: Download installer
# Visit: https://ollama.com/download

# Option B: Use winget
winget install Ollama.Ollama
```

### Step 2: Install a Translation Model
```powershell
# Recommended: Fast and good quality (2GB RAM)
ollama pull llama3.2:3b

# Or: Better quality (5GB RAM)
ollama pull llama3.1:8b

# Or: Excellent for Asian languages (4.5GB RAM)
ollama pull qwen2.5:7b
```

### Step 3: Test Ollama
```powershell
ollama run llama3.2:3b "Translate to Chinese: Hello world"
```

### Step 4: Configure GPTTrans
Create or edit `config.json`:
```json
{
  "openai_api_key": "",
  "openai_model": "llama3.2:3b",
  "target_lang": "English",
  "hotkey": "Alt+F3",
  "api_type": "ollama",
  "api_base": "http://localhost:11434/v1"
}
```

### Step 5: Build and Run GPTTrans
```powershell
cargo build --release
copy config.json target\release\
cd target\release
.\gpttrans.exe
```

**That's it!** 🎉 You now have:
- ✅ **100% FREE** translations (no API costs)
- ✅ **100% PRIVATE** (runs entirely on your computer)
- ✅ **No rate limits**
- ✅ **Works offline**
- ✅ **No chat history** stored anywhere

## 🔨 Build & Run (General)

1. **Build the app**:
   ```powershell
   cd gpttrans
   cargo build --release
   ```

2. **Configure**:
   - Copy `config.example.json` to `target\release\config.json`
   - Edit the config file (or use Settings UI after launch)

3. **Launch**:
   ```powershell
   .\target\release\gpttrans.exe
   ```

## 📖 Usage

1. **Translate Text**:
   - Copy any text to clipboard
   - Press your hotkey (default: `Alt+F3`)
   - Translation appears in a window and is copied to clipboard

2. **Configure Settings**:
   - Right-click the tray icon → **Settings**
   - Edit API key, model, language, hotkey, etc.
   - Click **Save** (requires restart to apply hotkey changes)

3. **View Translation Window**:
   - Left-click the tray icon to show/hide the translation window
   - Press `Esc` to hide the window
   - Click the **Copy** button to copy text again

4. **Exit**:
   - Right-click tray icon → **Quit**

## 🔐 Security & Privacy

### Using Ollama (Local AI)
- **100% Private**: All processing happens on your computer
- **No data leaves your machine**: Translations never touch external servers
- **No chat history**: Everything is ephemeral and temporary
- **No account required**: No sign-up, no tracking

### Using OpenAI API
- Your API key is read from `config.json` and sent directly to OpenAI over HTTPS (rustls-tls)
- No additional persistence or logging of your API key
- Translations are processed by OpenAI's servers (subject to their privacy policy)
- No chat history is maintained by GPTTrans (but may appear in OpenAI's logs)

## 🔧 Troubleshooting

### Hotkey doesn't trigger
- **Cause**: Another app is using the same hotkey
- **Solution**: Change the hotkey in Settings (e.g., try `Ctrl+Shift+T` or `Win+Q`)

### "Missing API key" error with Ollama
- **Cause**: `api_type` is not set to `ollama`
- **Solution**: In Settings, set API Type to `ollama` and save

### Ollama connection error
- **Cause**: Ollama is not running or wrong port
- **Solution**: 
  ```powershell
  # Check if Ollama is running
  ollama list
  
  # Restart Ollama service if needed
  # (It usually auto-starts after installation)
  ```

### Translations are slow with Ollama
- **Cause**: Large model or CPU-only mode
- **Solution**: 
  - Use a smaller model: `llama3.2:3b` instead of `llama3.1:8b`
  - Ensure GPU acceleration is enabled (Ollama auto-detects)
  - Close other resource-intensive apps

### Toast notifications don't appear
- **Cause**: Windows notifications disabled
- **Solution**: Enable notifications in Windows Settings → System → Notifications

### Window doesn't show after hotkey press
- **Cause**: Window might be off-screen or minimized
- **Solution**: Left-click the tray icon to restore the window

## 🔮 Other OpenAI-Compatible APIs

You can use any OpenAI-compatible API endpoint:

### LM Studio (Local, Free)
```json
{
  "api_type": "openai-compatible",
  "api_base": "http://localhost:1234/v1",
  "openai_model": "your-model-name",
  "openai_api_key": ""
}
```

### OpenRouter (Cloud, Pay-as-you-go)
```json
{
  "api_type": "openai-compatible",
  "api_base": "https://openrouter.ai/api/v1",
  "openai_model": "meta-llama/llama-3.1-8b-instruct:free",
  "openai_api_key": "sk-or-v1-..."
}
```

### LocalAI (Self-hosted)
```json
{
  "api_type": "openai-compatible",
  "api_base": "http://localhost:8080/v1",
  "openai_model": "your-model",
  "openai_api_key": ""
}
```

## 📝 Known Limitations

- **Single-instance**: Multiple instances can run simultaneously (not yet enforced)
- **Hotkey conflicts**: If another app uses your hotkey, it won't work
- **Model restart**: Changing hotkey requires app restart

## 🎯 Recommended Models

### For Ollama (Local, Free)

| Model | Size | RAM | Quality | Best For |
|-------|------|-----|---------|----------|
| `llama3.2:3b` | 2GB | 4GB+ | Good | Fast translations, limited resources |
| `llama3.1:8b` | 5GB | 8GB+ | Better | General use, balanced quality/speed |
| `qwen2.5:7b` | 4.5GB | 8GB+ | Excellent | Asian languages (Chinese, Japanese, Korean) |
| `mistral:7b` | 4.1GB | 8GB+ | Good | European languages |

**Installation**:
```powershell
ollama pull llama3.2:3b
```

### For OpenAI API (Paid)

| Model | Speed | Quality | Cost | Best For |
|-------|-------|---------|------|----------|
| `gpt-4o-mini` | Fast | Good | Low | Most translations |
| `gpt-4o` | Medium | Excellent | Medium | Complex texts |
| `gpt-4-turbo` | Medium | Excellent | High | Professional work |

### For OpenRouter (Pay-as-you-go)

- `meta-llama/llama-3.1-8b-instruct:free` - FREE tier
- `google/gemini-flash-1.5` - Fast and cheap
- `anthropic/claude-3.5-sonnet` - Highest quality

## 💡 Tips

- **Start with Ollama `llama3.2:3b`** - It's free, fast, and good enough for most translations
- **Use GPU** - Ollama automatically uses your GPU if available (much faster)
- **Adjust model size** - Smaller models are faster but less accurate; larger models are slower but better
- **Test different models** - Translation quality varies by language pair
- **Keep Ollama updated** - `ollama version` to check, reinstall to update

## 🤝 Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues.

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

**Made with ❤️ in Rust** | **Powered by OpenAI API or Ollama** | **100% Free Option Available**

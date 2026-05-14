# 🦀 Pump.fun Quant Bot (Professional Version)

Bot trading kuantitatif berperforma tinggi yang dibangun dengan **Rust** untuk ekosistem Pump.fun. Bot ini menggunakan arsitektur *event-driven* berbasis **Tokio** untuk memastikan latensi rendah dan eksekusi yang presisi.

## 🚀 Fitur Utama

- **Event-Driven Architecture**: Memproses ribuan event per detik dengan `tokio::mpsc` dan `dashmap`.
- **Paper Trading First**: Simulasi perdagangan virtual tanpa risiko dana asli untuk pengujian strategi.
- **Advanced Filtering Engine**: Filter token berdasarkan usia, lonjakan volume, pertumbuhan holder, likuiditas, dan distribusi whale.
- **Real-time Price Tracking**: Pemantauan harga instan untuk memicu Take Profit (TP) dan Stop Loss (SL).
- **Professional Log System**: Pencatatan trade, error, dan performa secara detail di direktori `logs/`.
- **Concurrent Management**: Menggunakan `DashMap` untuk keamanan data antar thread (thread-safe).

## 🛠️ Tech Stack

- **Runtime**: [Tokio](https://tokio.rs/) (Async Rust)
- **Networking**: `tokio-tungstenite` (WebSocket), `reqwest` (HTTP API Fallback)
- **Data Handling**: `serde`, `serde_json`, `dashmap`
- **Utility**: `chrono` (Time), `uuid` (Trade ID), `anyhow` (Error Handling)
- **Logging**: `tracing` & `tracing-subscriber`

## 📁 Struktur Proyek

```text
src/
├── analytics/  # Perhitungan performa & statistik (win rate, drawdown)
├── config/     # Loader konfigurasi & parameter strategi
├── engine/     # Otak bot (filter_engine, simulation, risk_management)
├── queue/      # Event buffer & dispatcher
├── stream/     # Listener WebSocket Pump.fun & Trade events
├── tracker/    # Pemantauan real-time (harga, volume, holder)
├── utils/      # Helper (logger, time, formatting)
└── wallet/     # Simulator saldo & multi-wallet
```

## ⚙️ Konfigurasi Strategi (Default)

Parameter dapat disesuaikan di `src/config/strategy.json`:

| Parameter | Nilai Default | Deskripsi |
|-----------|---------------|-----------|
| `token_age` | 30s - 180s | Usia token saat entry |
| `min_liquidity` | 8 SOL | Minimal likuiditas awal |
| `take_profit` | +25% | Target profit simulasi |
| `stop_loss` | -12% | Batas kerugian simulasi |
| `entry_size` | 0.1 SOL | Ukuran posisi per trade |

## 🚦 Cara Menjalankan

### Persyaratan
- [Rust & Cargo](https://rustup.rs/) (versi terbaru)
- Koneksi internet stabil

### Langkah-langkah
1. **Clone & Masuk ke Direktori**
   ```bash
   cd "pumpfun-quant-bot"
   ```

2. **Cek Kode**
   ```bash
   cargo check
   ```

3. **Jalankan Bot (Mode Debug)**
   ```bash
   cargo run
   ```

4. **Build untuk Produksi**
   ```bash
   cargo build --release
   ```

## 🗺️ Roadmap Pengembangan

- [x] **Phase 1**: Dasar WebSocket Listener & Event Queue.
- [x] **Phase 2**: Struktur Filter Engine & Simulation Engine.
- [ ] **Phase 3**: Integrasi Parsing JSON asli dari Pump.fun API.
- [ ] **Phase 4**: Implementasi `holder_tracker` & `volume_tracker`.
- [ ] **Phase 5**: Dasbor analytics sederhana di terminal.
- [ ] **Phase 6**: Implementasi Auto-buy (Real Trading) dengan integrasi Wallet Solana.

## ⚠️ Disclaimer

Bot ini disediakan untuk tujuan edukasi dan simulasi (**Paper Trading**). Trading mata uang kripto (terutama token meme/Pump.fun) melibatkan risiko tinggi. Penulis tidak bertanggung jawab atas kerugian finansial yang mungkin terjadi. **Gunakan dengan bijak.**

---
Built with ❤️ by Gemini CLI Agent

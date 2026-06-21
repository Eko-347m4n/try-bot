# Analisis Perdagangan Bot PumpFun Quant

## Database SQLite (`trades.db`)

### Struktur Tabel

| Table | Records | Fungsi |
|-------|---------|--------|
| `trades` | 129,627 | Riwayat trade dengan entry/exit price, pnl_pct, exit_type, strategy_id |
| `decision_traces` | 2,972,434 | Log keputusan filter per token |
| `window_stats` | 511 | Statistik window scan (pass_rate, win_rate_30, market_mode) |
| `virtual_topups` | 2 | Topup hanya untuk Legacy (2x 1 SOL) |
| `open_positions` | 0 | Tidak ada posisi terbuka |

### Kolom Tabel `trades`

- `id` — INTEGER PRIMARY KEY
- `timestamp` — TEXT
- `token_addr` — TEXT
- `entry_price` — REAL
- `exit_price` — REAL
- `pnl_pct` — REAL
- `exit_type` — TEXT (TakeProfit, StopLoss, TimeoutStale, TP, SL, STALE, ORPHAN)
- `hold_secs` — INTEGER
- `volume_entry` — REAL
- `velocity_score` — REAL
- `buyers_count` — INTEGER
- `entry_score` — REAL
- `hour_utc` — INTEGER
- `strategy_id` — TEXT (Alpha, Bravo, Charlie, Delta, Foxtrot, Legacy)

---

## Log Files

22 file log dari `bot.log.2026-05-29` sampai `bot.log.2026-06-19`.

Strategi dalam log menggunakan nama kode:
- Alpha (A)
- Bravo (B)
- Charlie (C)
- Delta (D)
- Foxtrot/Foxtrot (F)

### Temuan dari Log `bot.log.2026-06-19`

**Semua strategi kehabisan SOL:**

| Strategy | Balance | Cost | Status |
|----------|---------|------|--------|
| Alpha | 0.040 SOL | 0.050 SOL | INSUFFICIENT |
| Bravo | 0.048 SOL | 0.050 SOL | INSUFFICIENT |
| Charlie | 0.047 SOL | 0.053 SOL | INSUFFICIENT |
| Delta | 0.047 SOL | 0.053 SOL | INSUFFICIENT |
| Foxtrot | 0.047 SOL | 0.050 SOL | INSUFFICIENT |

Foxtrot masih membuat 415 keputusan BUY pada 19 Juni, tapi tidak ada trade yang tereksekusi. 5,068 keputusan REJECT karena filter (VolumeFilter, BuyersFilter, dll).

### Kronologi dari Log `bot.log.2026-06-18`

- **00:15:59** — Alpha (0.040), Bravo (0.048), Charlie (0.047), Delta (0.047) mulai gagal beli
- **00:16:14** — Foxtrot masih trading: Balance = 0.133 → 0.125 → 0.120 → 0.070 → 0.052 → 0.092 → 0.112 → 0.106 → 0.101 → 0.084
- Sepanjang hari Alpha/Bravo/Charlie/Delta balance tidak berubah (stuck)
- Foxtrot hanya 243 trade (vs normal 2,000-2,500/hari)
- Filter yang sering REJECT: `VolumeFilter` (Vol < 3.00 SOL), `BuyersFilter`

---

## BUG KRITIS: Perhitungan PnL% di Database

Database `trades.pnl_pct` mencatat angka tidak realistis:
- PnL per trade berkisar 2,601% — 13,009,637%
- **100% win rate** (semua trade positif)
- Bahkan trade `StopLoss` dengan `exit_price < entry_price` tetap tercatat positif

Contoh verifikasi:

```
entry=0.00015148, exit=0.00013907 → sebenarnya -8.19%, tercatat +30,202%
entry=0.00003512, exit=0.00003101 → sebenarnya -11.70%, tercatat +125,606%
```

**Perhitungan PnL di database salah** dan tidak bisa dipakai untuk evaluasi.

---

## Kinerja Real (dihitung ulang dari entry_price vs exit_price)

### Perbandingan Semua Strategi

| Strategy | Trades | Wins | Losses | WR | Avg PnL/Trade | Total PnL | Avg Hold |
|----------|--------|------|--------|----|---------------|-----------|----------|
| **Foxtrot** | **28,034** | **9,170** | **18,864** | **32.7%** | **+0.31%** | **+8,571.77%** | **10.4s** |
| Alpha | 31,675 | 8,609 | 23,066 | 27.2% | -1.24% | -39,334% | 49.0s |
| Bravo | 32,261 | 8,630 | 23,631 | 26.8% | -1.25% | -40,180% | 50.7s |
| Charlie | 18,722 | 4,303 | 14,419 | 23.0% | -1.05% | -19,736% | 40.9s |
| Delta | 18,722 | 4,303 | 14,419 | 23.0% | -1.05% | -19,736% | 40.9s |
| Legacy | 213 | 57 | 156 | 26.8% | -1.70% | -363% | 32.7s |

**Foxtrot adalah satu-satunya strategi dengan Expected Value positif secara agregat historis, namun EV-nya sedang menurun dan mungkin hanya positif di kondisi pasar tertentu.**

### Kinerja Harian Foxtrot (Real)

| Date | Trades | Wins | Losses | WR | Avg PnL% | Total PnL% |
|------|--------|------|--------|----|----------|------------|
| 2026-06-18 | 243 | 88 | 155 | 36.2% | +0.77% | +186.55% |
| 2026-06-17 | 2,053 | 560 | 1,493 | 27.3% | -0.83% | -1,698.02% |
| 2026-06-16 | 2,574 | 785 | 1,789 | 30.5% | -0.62% | -1,605.38% |
| 2026-06-15 | 2,424 | 804 | 1,620 | 33.2% | +0.47% | +1,149.27% |
| 2026-06-14 | 1,952 | 609 | 1,343 | 31.2% | -0.09% | -173.67% |
| 2026-06-13 | 2,415 | 815 | 1,600 | 33.7% | -0.10% | -233.79% |
| 2026-06-12 | 2,520 | 815 | 1,705 | 32.3% | +0.25% | +628.67% |
| 2026-06-11 | 2,517 | 835 | 1,682 | 33.2% | -0.23% | -577.72% |
| 2026-06-10 | 2,213 | 753 | 1,460 | 34.0% | +1.68% | +3,716.96% |
| 2026-06-09 | 1,950 | 662 | 1,288 | 33.9% | +0.37% | +728.17% |
| 2026-06-08 | 2,700 | 900 | 1,800 | 33.3% | +1.01% | +2,734.47% |
| 2026-06-07 | 2,524 | 870 | 1,654 | 34.5% | +0.75% | +1,887.88% |
| 2026-06-06 | 1,949 | 674 | 1,275 | 34.6% | +0.94% | +1,828.37% |

---

## Simulasi Balance Foxtrot

### Full History (28,034 trade, start 1 SOL)

| Metric | Value |
|--------|-------|
| Trades executed | 28,034 |
| Win Rate | 32.9% |
| Starting balance | 1.000 SOL |
| Final balance | **5.286 SOL** |
| Peak balance | 7.536 SOL |
| Min balance | 0.999 SOL |
| Net PnL | **+428.59%** |

### Last 5,000 Trades (start 1 SOL)

| Metric | Value |
|--------|-------|
| Trades executed | 2,961 |
| Win Rate | 30.4% |
| Runs out at trade | 2,961 |
| Final balance | **0.045 SOL** |
| Net PnL | **-95.47%** |

---

## Kesimpulan

### Strategi Terbaik

**Foxtrot (Strategi F) adalah satu-satunya strategi yang profitabel secara rata-rata per trade (+0.31%) bila dilihat dari seluruh histori.** Namun, data 11–17 Juni menunjukkan tren degradasi:

| Periode | Win Rate | Avg PnL/Trade | EV |
|---------|----------|---------------|----|
| Jun 6–10 | 33.9–34.6% | +0.37% s/d +1.68% | Positif |
| Jun 11–14 | 31.2–33.2% | -0.23% s/d -0.09% | Mendekati nol / negatif |
| Jun 15–17 | 27.3–33.2% | -0.83% s/d +0.47% | Negatif / mixed |

Win rate turun dari ~34% → ~27–30% — ini bukan noise, melainkan tren. **Kesimpulan yang lebih tepat: Foxtrot memiliki EV positif di kondisi pasar tertentu, bukan secara universal.** Semua strategi lain (Alpha/Bravo/Charlie/Delta) memiliki rata-rata PnL negatif. Legacy (yang asli) juga negatif.

### Kenapa Foxtrot "Rugi Total" di Sesi Terakhir?

1. **Drawdown berturut-turut**: Juni 17 (-1,698%), Juni 16 (-1,605%), Juni 14 (-174%), Juni 13 (-234%), Juni 11 (-578%)
2. **Modal habis**: Balance turun terus hingga 0.047 SOL (< minimum cost 0.050 SOL)
3. **Frekuensi trading tinggi** (2,000-2,500 trade/hari) mempercepat erosi modal saat losing streak
4. **Tidak ada risk management**: posisi size tetap (0.05 SOL), tidak ada stop-loss harian, tidak ada scaling down
5. **Tidak ada top-up modal** selama beroperasi (virtual_topups = 0 untuk non-Legacy)

### Rekomendasi

1. **Foxtrot perlu dipertahankan dengan syarat** — EV positif secara agregat, namun tren terbaru menunjukkan degradasi. Jangan asumsi EV selalu positif tanpa memantau perkembangan harian.
2. **Identifikasi kondisi pasar di mana Foxtrot positif/negatif** — analisis korelasi market regime (window_stats.market_mode) terhadap performa Foxtrot untuk menentukan kapan strategi aktif/di-pause.
3. **Tambahkan risk management**:
   - Cut loss harian (stop trading jika drawdown > X% dalam sehari)
   - Position sizing adaptif (turunkan ukuran saat drawdown)
   - Daily loss limit
   - Pause trading otomatis saat win rate rolling 7-hari turun di bawah 30%
4. **Top-up modal** secara periodik untuk menjaga balance di atas minimum
5. **Fix bug PnL%** di database agar perhitungan benar: `pnl_pct = (exit_price - entry_price) / entry_price * 100`
6. **Evaluasi filter** — 5,068 REJECT vs 415 BUY pada 19 Juni (Volume filter terlalu ketat?)

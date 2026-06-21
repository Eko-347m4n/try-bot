# Analisis Paper Trade — Apa yang Periksa

Setelah fix diterapkan dan paper trade berjalan 3+ hari, analisa dalam urutan ini:

---

## 1. Config Validation (Pertama, Bukan Terakhir)

Sebelum lihat PnL, konfirmasi dulu bot jalan dengan config yang benar.

**Cek dari startup log:**

```bash
grep "CONFIG" logs/bot.log.2026-06-* | grep -E "(Foxtrot|Alpha|Bravo)"
```

Output harus mengandung:

```
[Foxtrot] CONFIG | broker=[trading_fee=0% | priority_fee=0 SOL | net_roi=false]
[Alpha]   CONFIG | broker=[trading_fee=0% | priority_fee=0 SOL | net_roi=false]
[Charlie] CONFIG | broker=[trading_fee=1.25% | priority_fee=0.002 SOL | net_roi=true]
```

Setiap strategi punya fee yang sesuai dengan builder nya. **Jika ada yang nyangkut, stop — jangan lanjut sebelum diperbaiki.**

**Cek DB recording tidak drop:**

```bash
grep "DB channel full" logs/bot.log.2026-06-*
```

Jika ada baris ini, DB recording masih bermasalah.

---

## 2. Balance Integrity

Bandingkan balance akhir dari log dengan balance yang dihitung dari DB trades.

**Ekstrak dari log:**

```bash
# Balance akhir Foxtrot per hari
grep "Foxtrot" logs/bot.log.2026-06-21 | grep "Balance:" | tail -1
```

**Bandingkan dengan DB:**

```sql
SELECT 
  DATE(timestamp) as day,
  COUNT(*) as trades,
  ROUND(SUM(0.05 * (exit_price - entry_price) / entry_price), 4) as db_pnl
FROM trades
WHERE strategy_id = 'Foxtrot'
  AND DATE(timestamp) >= '2026-06-21'
GROUP BY day;
```

**Kriteria:**

| Selisih (log vs DB) | Arti |
|---------------------|------|
| < 0.01 SOL | ✅ Normal — trading fee=0 berfungsi |
| 0.01 – 0.10 SOL | ⚠️ Cek — mungkin ada rounding atau hidden cost |
| > 0.10 SOL | ❌ Ada fee tidak terduga — stop, investigasi |

Jika selisih > 0.10 SOL per hari, ada kemungkinan broker config nyangkut lagi atau fee mechanism tidak sesuai kode.

---

## 3. Distribusi PnL per Trade — Bukan Cuma Rata-rata

Query ini yang seharusnya dijalankan pertama kali setiap sesi:

```sql
SELECT 
  ROUND((exit_price - entry_price) / entry_price * 100, 1) as pnl_bucket,
  COUNT(*) as trades
FROM trades
WHERE strategy_id = 'Foxtrot'
  AND timestamp >= '2026-06-21'
GROUP BY pnl_bucket
ORDER BY pnl_bucket;
```

**Yang perlu diperhatikan:**

- **Apakah distribusi mirip dengan clean logs (June 6-13)?** Jika beda, market regime berubah.
- **Apakah ada outlier besar?** Trade dengan >+100% atau <-50% perlu ditandai dan diperiksa token address-nya.

**Cek manual outlier:**

```sql
SELECT token_addr, entry_price, exit_price, 
       (exit_price - entry_price) / entry_price * 100 as pnl,
       hold_secs, exit_type
FROM trades
WHERE strategy_id = 'Foxtrot'
  AND ABS((exit_price - entry_price) / entry_price) > 0.5  -- PnL >50% atau <-50%
ORDER BY ABS(pnl) DESC
LIMIT 10;
```

---

## 4. Fee Impact Aktual

Setelah ada kolom `gross_pnl_sol`, `fees_paid_sol`, `realized_net_sol`:

```sql
SELECT 
  DATE(timestamp) as day,
  COUNT(*) as trades,
  ROUND(SUM(gross_pnl_sol), 4) as gross,
  ROUND(SUM(fees_paid_sol), 4) as fees,
  ROUND(SUM(realized_net_sol), 4) as net,
  ROUND(SUM(fees_paid_sol) / NULLIF(SUM(gross_pnl_sol), 0) * 100, 1) as fee_pct_of_gross
FROM trades
WHERE strategy_id = 'Foxtrot'
  AND timestamp >= '2026-06-21'
GROUP BY day;
```

**Target:** `fee_pct_of_gross` harus 0% untuk strategi fee=0. Jika > 0%, ada biaya tidak terduga.

---

## 5. Variance Harian — Drawdown Terburuk

Dari log, ekstrak balance per hari:

```bash
python3 -c "
import re
with open('logs/bot.log.2026-06-21') as f:
    for line in f:
        if '[Foxtrot]' not in line: continue
        m = re.search(r'Balance:\s*([\d.]+)\s*SOL', line)
        if m: print(m.group(1))
" | sort -n | head -1  # min balance
" | sort -n | tail -1  # max balance
```

**Hitung:**
- Drawdown terbesar dalam satu hari (% dari start balance hari itu)
- Berapa hari yang loss?
- Apakah ada hari dengan drawdown > 15%?

Jika ada hari dengan drawdown > 15%, daily loss limit harus diaktifkan sebelum lanjut ke size yang lebih besar.

---

## 6. Exit Type Analysis

```sql
SELECT 
  exit_type,
  COUNT(*) as trades,
  ROUND(AVG((exit_price - entry_price) / entry_price * 100), 2) as avg_pnl,
  ROUND(AVG(hold_secs), 1) as avg_hold_s,
  ROUND(MIN((exit_price - entry_price) / entry_price * 100), 2) as worst,
  ROUND(MAX((exit_price - entry_price) / entry_price * 100), 2) as best
FROM trades
WHERE strategy_id = 'Foxtrot'
  AND timestamp >= '2026-06-21'
GROUP BY exit_type;
```

**Pertanyaan:**
- Apakah StopLoss lebih buruk dari -8% (sl_multiplier)? Jika ya, ada slippage.
- Apakah TakeProfit konsisten di +15-25%? Atau banyak yang cut early oleh trailing?
- TimeoutStale — apakah hold_secs rata-rata < 30s (timeout config)?

---

## 7. Winning vs Losing Days — Pattern Recognition

```python
# Untuk setiap hari di sesi baru:
# 1. Apakah loss day didahului oleh profit day besar? (mean reversion)
# 2. Apakah loss day terjadi beruntun?
# 3. Apakah ada korelasi dengan jumlah trade?

# Jika loss day terjadi setelah 3+ profit day beruntun:
#   → Sifat strategi mean-reverting, perlu take-profit period
# Jika loss day terjadi saat jumlah trade menurun:
#   → Likuiditas pasar turun, filter perlu diperketat
# Jika loss day tidak punya pola:
#   → Random variance — acceptable, but need circuit breaker
```

---

## 8. Metrik Ringkasan — Satu Halaman

Setelah 3 hari paper trade, hitung:

| Metrik | Rumus | Target |
|--------|-------|--------|
| Win Rate | wins / total | > 32% |
| Avg Win | avg of positive trades | > +20% |
| Avg Loss | avg of negative trades | > -15% (less negative) |
| Profit Factor | gross_win / gross_loss | > 1.2 |
| Sharpe (daily) | avg_daily_return / std_daily_return | > 1.0 |
| Max Drawdown | max peak-to-trough | < 20% |
| Fee Leakage | fee / gross_pnl | < 1% untuk fee=0 config |

Jika semua metrik di atas terpenuhi, strategi layak dipertimbangkan untuk size yang lebih besar.

---

## Urutan Analisis (Saran Eksekusi)

```
Step 1: Config validation        → 1 menit (grep log)
Step 2: Balance integrity        → 2 menit (query sqlite)
Step 3: PnL distribution         → 3 menit (query sqlite + visual)
Step 4: Variance harian          → 2 menit (log parsing)
Step 5: Exit type analysis       → 2 menit (query sqlite)
Step 6: Ringkasan metrik         → 3 menit (python aggregation)
──────────────────────────────────────
Total: ~13 menit per sesi evaluasi
```

Jika di step 1 atau 2 ada anomali, stop dan fix dulu sebelum lanjut.

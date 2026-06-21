# Fix Checklist — Sebelum Paper Trading Lagi

Berdasarkan temuan dari analisis June 6–20.

---

## Blocker: DB Recording Mati

**Temuan:** June 20 Foxtrot 1,540 trade, semua strategi 5,707 trade — 0 tersimpan di DB.

**Akar masalah:** `instance.rs:252` — `try_send` silent drop saat channel penuh.

```rust
// instance.rs:252 — saat ini:
let _ = trace_tx.try_send(TraceRecord::Trade(trade_record));
// ^ error diabaikan, trade hilang tanpa jejak
```

**Fix wajib:**

```rust
// Ganti jadi: minimal warn log saat drop
if let Err(e) = trace_tx.try_send(TraceRecord::Trade(trade_record)) {
    tracing::warn!(
        "[{}] DB channel full — trade DROPPED: {} ({:?}) | queue_remaining={}",
        self.strategy_id, token_address, decision, trace_tx.capacity() - trace_tx.len()
    );
}
```

Juga di `batch_worker.rs` — tambah metric monitoring:
- `trace_rx.len()` — antrian saat ini
- `total_received` / `total_written` — deteksi kalau batch worker stuck

---

## Critical: Startup Config Log

**Temuan:** Foxtrot jalan dengan broker config Charlie (fee=0.002+1.25%) di June 14, tapi builder.rs bilang fee=0. Tidak terdeteksi karena broker config tidak pernah di-log.

**Fix:**

1. Tambah method ke trait `Broker`:

```rust
// src/broker/simulator.rs
pub trait Broker: Send + Sync {
    fn calculate_entry(&self, entry_price: f64, size_sol: f64) -> (f64, f64);
    fn calculate_net_return(&self, exit_type: &ExitDecision, ...) -> f64;
    fn describe(&self) -> String;  // BARU
}
```

2. Implementasi:

```rust
// src/broker/simulator.rs
impl Broker for RealisticBroker {
    fn describe(&self) -> String {
        format!(
            "trading_fee={}% | priority_fee={} SOL | slippage_tp={}% | slippage_sl={}% | net_roi={}",
            self.trading_fee_rate * 100.0,
            self.priority_fee_sol,
            self.slippage_tp * 100.0,
            self.slippage_sl * 100.0,
            self.net_roi_enabled
        )
    }
    // ... existing methods unchanged
}
```

3. Log di `build_all` atau `main.rs`:

```rust
// main.rs — setelah build_all
for strategy in &strategies {
    info!("[{}] CONFIG | broker=[{}] | exit=[{:?}] | wallet={:.3} SOL",
        strategy.id(),
        strategy.broker.describe(),
        strategy.exit.get_tp_sl(),
        strategy.wallet.balance,
    );
}
```

Output yang diharapkan:

```
[Foxtrot] CONFIG | broker=[trading_fee=0% | priority_fee=0 SOL | net_roi=false] | exit=[TP=1.15, SL=0.92] | wallet=1.000 SOL
```

Setiap startup — langsung ketahuan kalau config nyangkut.

---

## High: Tambah Kolom Realized PnL di DB

**Temuan:** DB menyimpan `entry_price` dan `exit_price` (harga pasar), tapi `pnl_pct` dihitung dari `net_return` vs `cost`. Tidak ada kolom untuk fee aktual. Akibatnya: tidak bisa bedakan antara "rugi karena market" vs "rugi karena fee".

**Fix:**

```sql
ALTER TABLE trades ADD COLUMN gross_pnl_sol REAL;
ALTER TABLE trades ADD COLUMN fees_paid_sol REAL;
ALTER TABLE trades ADD COLUMN realized_net_sol REAL;
```

Di `close_position` (`instance.rs:225`):

```rust
let gross_pnl = size_sol * (current_price / entry_price) - size_sol;
let fees_paid = (cost - size_sol) + (size_sol * (current_price / entry_price) - net_return);
// cost - size_sol = buy-side fees
// (gross_return - net_return) = sell-side fees

let trade_record = TradeTrace {
    // ... existing fields ...
    // tambah:
    gross_pnl_sol: gross_pnl,
    fees_paid_sol: fees_paid,
    realized_net_sol: pnl,  // existing variable
};
```

Ini memungkinkan query:

```sql
SELECT DATE(timestamp),
       SUM(gross_pnl_sol) as gross,
       SUM(fees_paid_sol) as fees,
       SUM(realized_net_sol) as net
FROM trades WHERE strategy_id = 'Foxtrot'
GROUP BY 1;
```

---

## Medium: Daily Loss Limit di VirtualWallet

**Temuan:** Tidak ada `daily_start_balance` — tidak bisa deteksi drawdown harian.

**Fix:**

```rust
// src/strategy/instance.rs
pub struct VirtualWallet {
    pub balance: f64,
    pub realized_pnl: f64,
    pub trade_count: u32,
    pub tp_hits: u32,
    pub sl_hits: u32,
    pub daily_start_balance: f64,   // BARU
    pub last_reset_day: u32,        // BARU — UTC date as YYYYMMDD
}
```

Reset di `execute_buy`:

```rust
fn execute_buy(&mut self, ...) {
    let today = Utc::now().format("%Y%m%d").parse::<u32>().unwrap();
    if today != self.wallet.last_reset_day {
        self.wallet.daily_start_balance = self.wallet.balance;
        self.wallet.last_reset_day = today;
    }

    // Cek daily loss
    let daily_drawdown = (self.wallet.daily_start_balance - self.wallet.balance) / self.wallet.daily_start_balance;
    if daily_drawdown > 0.10 {  // 10%
        tracing::warn!("[{}] DAILY LOSS LIMIT: {:.1}% — stop trading", self.id(), daily_drawdown * 100.0);
        return;  // skip buy
    }

    // ... existing buy logic ...
}
```

Ini circuit breaker sederhana sebelum implementasi penuh.

---

## Medium: Runtime Fee Auditor

Di background task (satu jam sekali atau tiap 100 trade), hitung:

```rust
// Pseudo:
let expected_pnl = sum of (0.05 * (exit/entry - 1)) from DB for today;
let actual_change = current_wallet_balance - daily_start_balance;
let hidden_fees = expected_pnl - actual_change;

if hidden_fees.abs() > expected_pnl.abs() * 0.05 {  // deviasi > 5%
    warn!("[{}] FEE ANOMALY: expected={:.4}, actual={:.4}, hidden={:.4}",
          id, expected_pnl, actual_change, hidden_fees);
    // Kirim alert via Telegram
}
```

---

## Prioritas Eksekusi

| # | Item | Efek jika tidak di-fix |
|---|------|------------------------|
| P0 | DB recording (warn on drop) | Semua analisis buta — data hilang |
| P1 | Startup config log | Config nyangkut tidak terdeteksi |
| P2 | Kolom realized fee di DB | Tidak bisa bedakan market loss vs fee loss |
| P3 | Daily loss limit | Drawdown harian -176% dalam 1 hari = margin call |
| P4 | Runtime fee auditor | Deviasi fee tidak ketahuan sampai report harian |

---

## Ceklist Sebelum Paper Trade

- [ ] DB recording — `try_send` error di-log, bukan silent
- [ ] Startup config — semua strategi log broker config
- [ ] Kolom `gross_pnl_sol`, `fees_paid_sol` ada di DB (atau siap)
- [ ] VirtualWallet punya `daily_start_balance` dan reset logic
- [ ] Satu paper trade session (3 hari) berjalan tanpa silent data loss

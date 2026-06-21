# Solusi Perbaikan Bug dan Manajemen Risiko Strategi Foxtrot

Berdasarkan analisis performa bot trading pada [analisis-trading-bot.md](file:///home/yi/try-bot/doc/analisis-trading-bot.md), strategi **Foxtrot (Strategi F)** merupakan satu-satunya strategi yang memiliki Expected Value (EV) positif secara agregat historis (+0.31% per trade). **Namun, data 11–17 Juni menunjukkan tren degradasi** — win rate turun dari ~34% ke ~27–30% dan EV mendekati nol atau negatif. EV positif Foxtrot bersifat kondisional (bergantung regime pasar), bukan universal. Dua masalah utama menyebabkan strategi ini menderita kerugian total (wipeout) di akhir sesi:
1. **Bug kritis** pada kalkulasi persentase PnL di database.
2. **Ketiadaan manajemen risiko** yang memadai saat menghadapi rentetan kerugian (*drawdown/losing streak*).

Berikut adalah solusi lengkap dan detail implementasinya untuk mengatasi kedua masalah tersebut.

---

## 1. Perbaikan Bug Kritis PnL% di Database (Telah Diperbaiki)

### A. Deskripsi Bug
Pada file [instance.rs](file:///home/yi/try-bot/src/strategy/instance.rs#L225-L235), terdapat kesalahan dalam pendeklarasian variabel saat membongkar (*destructuring*) tuple hasil dari fungsi `self.broker.calculate_entry`.

Fungsi `calculate_entry` di [simulator.rs](file:///home/yi/try-bot/src/broker/simulator.rs#L22-L33) didefinisikan sebagai berikut:
```rust
fn calculate_entry(&self, entry_price: f64, size_sol: f64) -> (f64, f64) {
    // ...
    // Mengembalikan: (effective_entry_price, total_buy_cost)
    (entry_price, total_buy_cost)
}
```

Namun pada kode lama [instance.rs](file:///home/yi/try-bot/src/strategy/instance.rs) line 230:
```rust
let (cost, _) = self.broker.calculate_entry(entry_price, size_sol);
let pnl = net_return - cost;
let pnl_pct = (pnl / cost) * 100.0;
```
Kode di atas memetakan elemen pertama tuple (`effective_entry_price`) ke dalam variabel `cost`, bukan elemen kedua (`total_cost_sol`).

### B. Dampak Bug
- **Persentase PnL Salah Total**: Pembagi dalam rumus persentase PnL adalah `cost`. Karena `cost` berisi harga token (misalnya `0.00015148 SOL`), bukan ukuran SOL yang ditradingkan (misalnya `0.05 SOL`), pembagi menjadi sangat kecil.
- **Win Rate 100% Palsu**: Nilai `pnl_pct` yang dihasilkan berkisar antara `+2,600%` hingga `+13,000,000%`. Hal ini membuat trade yang sebenarnya rugi (*Stop Loss*) tetap tercatat positif di database karena nilai `net_return` (misal `0.045 SOL`) jauh lebih besar daripada harga token itu sendiri (`0.00015148 SOL`).
- **Verifikasi Perhitungan**:
  - *Sebelum perbaikan*: entry = 0.00015148, exit = 0.00013907, size = 0.05 SOL.
    $$\text{net\_return} = 0.0459\text{ SOL}$$
    $$\text{cost (salah)} = 0.00015148\text{ SOL}$$
    $$\text{pnl} = 0.0459 - 0.00015148 = 0.04575\text{ SOL}$$
    $$\text{pnl\_pct} = (0.04575 / 0.00015148) \times 100\% = +30,202\%$$

### C. Solusi Perbaikan Kode
Kami telah memperbaiki kesalahan destrukturisasi tersebut pada file [instance.rs](file:///home/yi/try-bot/src/strategy/instance.rs#L230):

```diff
-            let (cost, _) = self.broker.calculate_entry(entry_price, size_sol);
+            let (_, cost) = self.broker.calculate_entry(entry_price, size_sol);
             let pnl = net_return - cost;
             let pnl_pct = (pnl / cost) * 100.0;
```

Dengan perbaikan ini, variabel `cost` akan menyimpan `total_buy_cost` secara benar (~0.05 SOL), sehingga perhitungan PnL% di database kembali akurat dan mencerminkan kinerja realitas.

---

## 2. Solusi Manajemen Risiko untuk Mengatasi Kerugian Sesi Akhir

Meskipun Foxtrot adalah strategi dengan EV positif secara agregat (+0.31% per trade), EV ini bersifat kondisional — data 11–17 Juni menunjukkan tren degradasi di mana win rate turun dan EV mendekati nol/negatif. Frekuensi trading yang tinggi (2,000–2,500 trade/hari) bertindak sebagai pedang bermata dua. Saat pasar memasuki regime yang tidak cocok untuk Foxtrot (drawdown beruntun pada 16–17 Juni), modal Foxtrot habis terkikis karena tidak ada mekanisme pertahanan.

Berikut adalah solusi arsitektural manajemen risiko yang direkomendasikan untuk diimplementasikan pada bot:

### A. Circuit Breaker Harian (Daily Loss Limit)
Untuk menghentikan erosi modal saat terjadi rentetan kerugian (*losing streak*), bot harus berhenti beraktivitas jika kerugian harian melebihi ambang batas tertentu.

- **Mekanisme**:
  - Simpan saldo awal harian (*daily starting balance*).
  - Setiap kali posisi ditutup, hitung total akumulasi kerugian hari itu.
  - Jika kerugian hari berjalan melebihi **15%** dari saldo awal hari tersebut, aktifkan mode *pause* otomatis untuk sisa hari tersebut.
  - Reset status ini saat pergantian hari (Midnight UTC).

### B. Adaptive Position Sizing (Dynamic Sizing)
Menggunakan ukuran transaksi statis (`0.05 SOL`) saat saldo menipis mempercepat kebangkrutan (*Risk of Ruin*). Bot harus mengurangi ukuran trade-nya secara dinamis.

- **Formula Usulan**:
  Daripada menggunakan nilai tetap `0.05 SOL`, gunakan persentase saldo berjalan:
  $$\text{Size SOL} = \min(\max(\text{Wallet Balance} \times 5\%, 0.02\text{ SOL}), 0.10\text{ SOL})$$
- **Keuntungan**:
  - Saat saldo berkurang akibat drawdown, ukuran posisi akan mengecil secara otomatis hingga seminimal mungkin (`0.02 SOL`), sehingga memperpanjang umur bot (daya tahan trading).
  - Saat saldo meningkat (winning streak), ukuran posisi meningkat hingga batas maksimal (`0.10 SOL`) untuk memaksimalkan profit.

### C. Auto-Topup & Alerting untuk Strategi Virtual
Strategi Legacy memiliki fitur auto-topup untuk menjaga saldo tetap di atas `0.05 SOL`. Fitur ini tidak ada pada strategi independen seperti Foxtrot.

- **Mekanisme**:
  - Tambahkan fitur monitoring saldo wallet per strategi pada [instance.rs](file:///home/yi/try-bot/src/strategy/instance.rs).
  - Jika saldo Foxtrot di bawah minimum (`0.05 SOL`), kirimkan notifikasi kritis via Telegram (`notifier.send_generic_alert`) agar pengguna mengetahui status kekurangan dana.
  - Opsional: Terapkan penambahan saldo virtual otomatis dengan mencatatnya sebagai `virtual_topup` di database seperti halnya pada strategi Legacy.

### D. Analisis Volume Filter — Jangan Longgarkan, Mungkin Diperketat

Rekomendasi awal (menurunkan VolumeFilter dari 3.0 SOL ke 1.5 SOL saat Cold) keliru arah. Pertimbangan:

- **Data tidak mendukung**: Pada 19 Juni, semua strategi sudah INSUFFICIENT balance — 0 dari 415 BUY tereksekusi. Tidak ada data PnL untuk menilai apakah filter terlalu ketat atau tidak.
- **Risiko PumpFun**: Volume rendah di PumpFun berkorelasi dengan higher pump-and-dump / rug risk. Melonggarkan filter justru meningkatkan eksposur ke token berbahaya di saat modal paling tipis.
- **Pertanyaan benar**: Apakah trade yang lolos filter profitable? Jika ya, filter tidak perlu diubah — masalahnya modal habis, bukan filter terlalu ketat.

**Mekanisme alternatif (survival mode)**:
- Jika saldo mendekati minimum (< 0.10 SOL), *perketat* filter sementara (misal VolumeFilter naik ke 5.0 SOL) untuk menghemat modal dan hanya mengambil trade berkualitas tertinggi.
- Jika balance pulih, kembalikan ke threshold normal.
- Logika ini konsisten dengan Adaptive Position Sizing: saat modal menipis, kurangi risk exposure — jangan tambah.

---

### E. Market-Regime Based Trading (Pause/Resume Otomatis)

Karena EV Foxtrot bersifat kondisional (positif hanya di regime pasar tertentu), bot perlu mendeteksi kapan harus berhenti trading.

- **Mekanisme**:
  - Gunakan `window_stats.market_mode` atau metrik rolling win rate 7-hari untuk menentukan regime pasar.
  - Jika rolling win rate 7-hari turun di bawah 30% atau `market_mode` menunjukkan kondisi *Cold/Quiet* berkepanjangan, pause trading Foxtrot secara otomatis.
  - Resume hanya ketika rolling win rate kembali di atas 32% atau market mode berubah.
- **Mengapa**: Ini mencegah strategi terus trading saat kondisi pasar tidak menguntungkan, mengingat EV positif Foxtrot tidak universal.

---

## 3. Rencana Langkah Aksi Implementasi Kode Selanjutnya

Untuk menerapkan solusi manajemen risiko di atas, berikut adalah perubahan yang dapat diintegrasikan ke codebase:

1. **Menambahkan Parameter Risiko di Konfigurasi**:
   Tambahkan opsi `max_daily_loss_pct` dan `dynamic_sizing_pct` pada [config.rs](file:///home/yi/try-bot/src/config/mod.rs) atau `StrategyInstance`.
   
2. **Modifikasi `execute_buy` pada `StrategyInstance`**:
   Ubah penentuan `size_sol` menjadi dinamis berdasarkan persentase saldo dompet berjalan:
   ```rust
   let balance = self.wallet.balance;
   let size_sol = ((balance * 0.05).max(0.02)).min(0.10);
   ```

3. **Menambahkan Pelacakan Saldo Harian**:
   Tambahkan field `daily_start_balance` dan `last_reset_timestamp` pada struct `VirtualWallet` untuk mendukung fungsionalitas Circuit Breaker.

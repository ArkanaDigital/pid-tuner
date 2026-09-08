import type { Step } from "../lib/types";

export interface Shortcut {
  id: string;
  label: string;
  prompt: string;
}

const common = {
  quality: { id: "quality", label: "Jelaskan kualitas log ini", prompt: "Jelaskan kualitas log ini (rate, durasi, hover, saturasi, segmen step/chirp, anomali) dan apakah cukup untuk langkah ini. Kutip angkanya." },
  guards: { id: "guards", label: "Kenapa guard gagal?", prompt: "Guard mana yang belum lolos di langkah ini, kenapa, dan apa yang harus saya lakukan supaya lolos? Jangan sarankan override kecuali memang tidak ada jalan lain." },
  anomalies: { id: "anomalies", label: "Ada anomali berbahaya?", prompt: "Cek anomali di log ini. Mana yang kritis, apa artinya secara fisik, dan apa yang harus dicek di hardware sebelum terbang lagi?" },
  review: (phase: "filter" | "PID") => ({ id: `review_${phase}`, label: `Tinjau rekomendasi ${phase}`, prompt: `Tinjau daftar rekomendasi ${phase} saat ini: apakah masuk akal dibanding data (puncak spektrum / step response / respons frekuensi)? Setujui yang kuat, tandai yang lemah, dan jelaskan alasannya per parameter.` }),
  conservative: { id: "conservative", label: "Lebih konservatif", prompt: "Buat versi rekomendasi yang lebih konservatif: perubahan lebih kecil, hanya yang didukung data kuat. Ubah nilai lewat set_recommendation atau tambahkan lewat add_recommendation, lalu ringkas apa yang kamu ubah." },
  aggressive: { id: "aggressive", label: "Lebih agresif", prompt: "Kalau data mendukung, usulkan langkah yang lebih agresif (tetap dalam batas parameter dan satu perubahan per parameter). Jelaskan risikonya dan tandai usulan sebagai belum diterima." },
  explain_step: { id: "explain_step", label: "Jelaskan step response", prompt: "Jelaskan step response tiap axis (overshoot, latency, steady state, jumlah segmen) dalam bahasa yang mudah untuk pilot, dan apa artinya untuk P/I/D/FF." },
  explain_freq: { id: "explain_freq", label: "Jelaskan respons frekuensi", prompt: "Jelaskan respons frekuensi CHIRP tiap axis (bandwidth, phase margin, resonant peak, koherensi) dan apa artinya untuk gain; sebutkan axis yang datanya tidak bisa dipercaya." },
  protocol: { id: "protocol", label: "Jelaskan protokol terbang", prompt: "Jelaskan protokol terbang untuk langkah ini dengan bahasa sederhana untuk pilot, termasuk kesalahan umum yang membuat datanya tidak bisa dipakai." },
  apply_summary: { id: "apply_summary", label: "Ringkas perubahan & risiko", prompt: "Ringkas parameter yang akan ditulis (nilai lama → baru), kenapa, risikonya, dan mana yang butuh reboot. Ingatkan apa yang harus diperhatikan di penerbangan berikutnya." },
  compare: { id: "compare", label: "Nilai before/after", prompt: "Bandingkan before/after (Flight B vs C): axis mana yang membaik, mana yang memburuk, dan apa langkah berikutnya. Kutip angkanya." },
  report_summary: { id: "report_summary", label: "Ringkasan untuk pilot", prompt: "Tulis ringkasan singkat untuk pilot: apa yang diubah, hasilnya, dan 3 hal yang harus diperhatikan di penerbangan berikutnya. Bahasa sederhana." },
  fc: { id: "fc", label: "Cek status FC", prompt: "Cek status flight controller dan konfigurasi logging-nya; apa yang belum siap untuk terbang data?" },
};

export const SHORTCUTS: Record<Step, Shortcut[]> = {
  connect: [common.fc, common.guards],
  preflight: [common.fc, common.guards, { id: "prep", label: "Apa yang harus disiapkan?", prompt: "Apa yang harus disiapkan sebelum penerbangan data pertama (logging, props, baterai, area)?" }],
  flight_a: [common.protocol],
  import_a: [common.quality, common.guards, common.anomalies],
  filter_analysis: [common.review("filter"), common.conservative, common.aggressive, { id: "peaks", label: "Puncak noise terpenting", prompt: "Puncak noise mana yang paling penting untuk ditangani, dan filter apa yang tepat (notch dinamis, RPM, LPF)? Kutip frekuensi dan levelnya." }],
  apply_filters: [common.apply_summary],
  flight_b: [common.protocol],
  import_b: [common.quality, common.guards, common.anomalies],
  pid_analysis: [common.review("PID"), common.conservative, common.aggressive, common.explain_step, common.explain_freq],
  apply_pids: [common.apply_summary],
  flight_c: [common.protocol],
  import_c: [common.quality, common.guards, common.anomalies],
  compare: [common.compare],
  report: [common.report_summary, common.compare],
};

export const QUICK_SHORTCUTS: Shortcut[] = [common.quality, common.review("filter"), common.review("PID"), common.anomalies, common.explain_step, common.explain_freq];

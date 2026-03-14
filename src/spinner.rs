// spinner.rs - 処理中スピナー（30x30 最前面表示）
//
// Markdown整形など時間のかかる処理中に、小さなプログレスインジケーターを表示する。
// Arc<AtomicBool> で完了シグナルを受け取り、自動的に閉じる。

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// スピナーウィンドウのサイズ
const SPINNER_SIZE: f32 = 36.0;

/// スピナーApp
struct SpinnerApp {
    done: Arc<AtomicBool>,
    start: Instant,
}

impl eframe::App for SpinnerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 完了シグナルで閉じる
        if self.done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let bg = egui::Color32::from_rgba_premultiplied(30, 30, 46, 230);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(bg))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();
                let center = rect.center();
                let radius = rect.width().min(rect.height()) * 0.35;

                let elapsed = self.start.elapsed().as_secs_f32();

                // 背景の円（トラック）
                let track_color = egui::Color32::from_rgb(69, 71, 90);
                ui.painter().circle_stroke(
                    center,
                    radius,
                    egui::Stroke::new(3.0, track_color),
                );

                // 回転するアーク（扇形の弧）
                let accent = egui::Color32::from_rgb(137, 180, 250);
                let segments = 20;
                let arc_len = std::f32::consts::PI * 1.2; // 約216度
                let base_angle = elapsed * 4.0; // 回転速度

                let points: Vec<egui::Pos2> = (0..=segments)
                    .map(|i| {
                        let t = i as f32 / segments as f32;
                        let angle = base_angle + t * arc_len;
                        egui::pos2(
                            center.x + radius * angle.cos(),
                            center.y + radius * angle.sin(),
                        )
                    })
                    .collect();

                for w in points.windows(2) {
                    ui.painter().line_segment(
                        [w[0], w[1]],
                        egui::Stroke::new(3.0, accent),
                    );
                }
            });

        // 高頻度リペイントでアニメーション
        ctx.request_repaint_after(Duration::from_millis(30));
    }
}

/// スピナーを表示する（別スレッドから呼ぶ）。done が true になると自動で閉じる。
pub fn show_spinner(done: Arc<AtomicBool>) {
    // カーソル位置の近くに表示
    let (x, y) = cursor_position();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([SPINNER_SIZE, SPINNER_SIZE])
            .with_position(egui::pos2(x as f32 - SPINNER_SIZE / 2.0, y as f32 - SPINNER_SIZE - 8.0))
            .with_decorations(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false),
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        })),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Lanch App Spinner",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(SpinnerApp {
                done,
                start: Instant::now(),
            }) as Box<dyn eframe::App>)
        }),
    );
}

/// 現在のカーソル位置を取得
fn cursor_position() -> (i32, i32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos};
        use windows_sys::Win32::Foundation::POINT;
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt); }
        (pt.x, pt.y)
    }
    #[cfg(not(windows))]
    {
        (500, 500)
    }
}

// spinner.rs - 処理中スピナー（Win32ネイティブウィンドウ）
//
// Markdown整形など時間のかかる処理中に、小さなプログレスインジケーターを表示する。
// Arc<AtomicBool> で完了シグナルを受け取り、自動的に閉じる。
//
// eframe::run_native は同一スレッドで2回呼べないため、Win32 API で直接描画する。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// スピナーウィンドウのサイズ
const SPINNER_SIZE: i32 = 40;

/// スピナーを表示する（別スレッドから呼ぶ）。done が true になると自動で閉じる。
pub fn show_spinner(done: Arc<AtomicBool>) {
    #[cfg(windows)]
    show_spinner_win32(done);

    #[cfg(not(windows))]
    {
        // 非Windows: 完了まで待つだけ
        while !done.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(windows)]
fn show_spinner_win32(done: Arc<AtomicBool>) {
    use windows_sys::Win32::Foundation::*;
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WM_TIMER: u32 = 0x0113;
    const TIMER_ID: usize = 1;
    const FRAME_MS: u32 = 33; // ~30fps

    // スピナー状態をスレッドローカルに保持
    thread_local! {
        static SPINNER_DONE: std::cell::RefCell<Option<Arc<AtomicBool>>> = const { std::cell::RefCell::new(None) };
        static SPINNER_ANGLE: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
        match msg {
            WM_TIMER => {
                // 完了チェック
                let should_close = SPINNER_DONE.with(|cell| {
                    cell.borrow().as_ref().map_or(true, |d| d.load(Ordering::SeqCst))
                });
                if should_close {
                    unsafe {
                        KillTimer(hwnd, TIMER_ID);
                        DestroyWindow(hwnd);
                    }
                    return 0;
                }
                // 角度を更新して再描画
                SPINNER_ANGLE.with(|a| a.set(a.get() + 0.15));
                unsafe { InvalidateRect(hwnd, std::ptr::null(), 1); }
                0
            }
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
                let hdc = unsafe { BeginPaint(hwnd, &mut ps) };

                let mut rc: RECT = unsafe { std::mem::zeroed() };
                unsafe { GetClientRect(hwnd, &mut rc); }
                let cx = (rc.right / 2) as f32;
                let cy = (rc.bottom / 2) as f32;
                let radius = cx.min(cy) * 0.7;

                // 背景
                let bg_brush = unsafe { CreateSolidBrush(0x002E1E1E) }; // dark bg (BGR)
                unsafe { FillRect(hdc, &rc, bg_brush); }
                unsafe { DeleteObject(bg_brush as _); }

                // 回転アーク
                let angle = SPINNER_ANGLE.with(|a| a.get());
                let accent_pen = unsafe { CreatePen(0, 3, 0x00FA8A89) }; // accent blue (BGR)
                let old_pen = unsafe { SelectObject(hdc, accent_pen as _) };

                let segments = 16;
                let arc_len = std::f32::consts::PI * 1.2;
                for i in 0..segments {
                    let t0 = i as f32 / segments as f32;
                    let t1 = (i + 1) as f32 / segments as f32;
                    let a0 = angle + t0 * arc_len;
                    let a1 = angle + t1 * arc_len;
                    unsafe {
                        MoveToEx(hdc, (cx + radius * a0.cos()) as i32, (cy + radius * a0.sin()) as i32, std::ptr::null_mut());
                        LineTo(hdc, (cx + radius * a1.cos()) as i32, (cy + radius * a1.sin()) as i32);
                    }
                }

                unsafe { SelectObject(hdc, old_pen); }
                unsafe { DeleteObject(accent_pen as _); }
                unsafe { EndPaint(hwnd, &ps); }
                0
            }
            WM_DESTROY => {
                unsafe { PostQuitMessage(0); }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    SPINNER_DONE.with(|cell| {
        *cell.borrow_mut() = Some(done);
    });
    SPINNER_ANGLE.with(|a| a.set(0.0));

    unsafe {
        let h_instance = GetModuleHandleW(std::ptr::null());
        let class_name: Vec<u16> = "LanchSpinner\0".encode_utf16().collect();

        let wc = WNDCLASSW {
            style: CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        RegisterClassW(&wc);

        // カーソル位置に配置
        let (cx, cy) = cursor_position();
        let x = cx - SPINNER_SIZE / 2;
        let y = cy - SPINNER_SIZE - 8;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            class_name.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_VISIBLE,
            x, y, SPINNER_SIZE, SPINNER_SIZE,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            h_instance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            eprintln!("[spinner] ウィンドウ作成失敗");
            return;
        }

        // 半透明
        SetLayeredWindowAttributes(hwnd, 0, 220, 0x02 /* LWA_ALPHA */);
        // 角丸
        let rgn = CreateRoundRectRgn(0, 0, SPINNER_SIZE, SPINNER_SIZE, 8, 8);
        SetWindowRgn(hwnd, rgn, 1);

        SetTimer(hwnd, TIMER_ID, FRAME_MS, None);
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg);
        }

        UnregisterClassW(class_name.as_ptr(), h_instance);
    }
}

/// 現在のカーソル位置を取得
fn cursor_position() -> (i32, i32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut pt = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut pt); }
        (pt.x, pt.y)
    }
    #[cfg(not(windows))]
    {
        (500, 500)
    }
}

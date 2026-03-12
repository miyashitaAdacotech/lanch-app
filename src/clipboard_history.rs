// clipboard_history.rs - クリップボード監視 & 履歴管理
//
// 責務:
// - Win32 AddClipboardFormatListener でクリップボード変更をリアルタイム監視
// - 変更検出時に arboard でテキスト/画像を読み取り、ClipboardStore に保存
// - 定期的にローテーション（1週間超のエントリを削除）
//
// アーキテクチャ:
// - バックグラウンドスレッドで隠しウィンドウを作成し、メッセージループを回す
// - Arc<Mutex<ClipboardStore>> で UI スレッドと共有

use std::sync::{Arc, Mutex};
use std::thread;

use crate::clipboard_store::ClipboardStore;

/// クリップボード履歴の保持日数（デフォルト: 7日）
const DEFAULT_RETENTION_DAYS: i64 = 7;

/// ローテーション間隔（秒）: 1時間ごとに古いエントリを削除
const ROTATION_INTERVAL_SECS: u64 = 3600;

/// 共有ストアの型エイリアス
pub type SharedStore = Arc<Mutex<ClipboardStore>>;

/// クリップボード監視を開始する
///
/// バックグラウンドスレッドを起動し、クリップボード変更を監視する。
/// 戻り値の SharedStore を clipboard_ui に渡して履歴UIから参照する。
pub fn start_monitoring() -> SharedStore {
    let store = ClipboardStore::new(DEFAULT_RETENTION_DAYS);
    let shared = Arc::new(Mutex::new(store));

    // ローテーション用スレッド
    let rotation_store = shared.clone();
    thread::spawn(move || {
        loop {
            thread::sleep(std::time::Duration::from_secs(ROTATION_INTERVAL_SECS));
            if let Ok(mut store) = rotation_store.lock() {
                store.rotate();
            }
        }
    });

    // クリップボード監視スレッド
    let monitor_store = shared.clone();
    thread::spawn(move || {
        run_clipboard_monitor(monitor_store);
    });

    shared
}

/// Windows: クリップボード監視メッセージループ
#[cfg(windows)]
fn run_clipboard_monitor(store: SharedStore) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // WM_CLIPBOARDUPDATE = 0x031D
    const WM_CLIPBOARDUPDATE: u32 = 0x031D;

    // ウィンドウプロシージャ用のグローバル状態
    // (ウィンドウプロシージャから SharedStore にアクセスするため)
    thread_local! {
        static THREAD_STORE: std::cell::RefCell<Option<SharedStore>> = const { std::cell::RefCell::new(None) };
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if msg == WM_CLIPBOARDUPDATE {
            THREAD_STORE.with(|cell| {
                if let Some(ref store) = *cell.borrow() {
                    on_clipboard_update(store);
                }
            });
            return 0;
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    // thread_local にストアをセット
    THREAD_STORE.with(|cell| {
        *cell.borrow_mut() = Some(store);
    });

    unsafe {
        let h_instance = GetModuleHandleW(std::ptr::null());

        // ウィンドウクラス登録
        let class_name: Vec<u16> = "LanchAppClipboardMonitor\0"
            .encode_utf16()
            .collect();

        let wc = WNDCLASSW {
            style: 0,
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

        if RegisterClassW(&wc) == 0 {
            eprintln!("[clipboard_history] ウィンドウクラスの登録に失敗");
            return;
        }

        // メッセージ専用ウィンドウを作成（HWND_MESSAGE = -3）
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            -3isize as HWND, // HWND_MESSAGE
            std::ptr::null_mut(),
            h_instance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            eprintln!("[clipboard_history] メッセージウィンドウの作成に失敗");
            return;
        }

        // クリップボード変更リスナーを登録
        if AddClipboardFormatListener(hwnd) == 0 {
            eprintln!("[clipboard_history] AddClipboardFormatListener に失敗");
            return;
        }

        eprintln!("[clipboard_history] クリップボード監視を開始しました");

        // メッセージループ
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// クリップボード変更時の処理
#[cfg(windows)]
fn on_clipboard_update(store: &SharedStore) {
    // arboard でクリップボード内容を読み取る
    let mut cb = match arboard::Clipboard::new() {
        Ok(cb) => cb,
        Err(_) => return,
    };

    // テキストを試行
    if let Ok(text) = cb.get_text() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            // 内部マーカーは無視（copy_selected_text のマーカー）
            if trimmed.starts_with("__QT_MARKER_") {
                return;
            }
            if let Ok(mut s) = store.lock() {
                s.add_text(trimmed);
            }
            return;
        }
    }

    // 画像を試行
    if let Ok(img) = cb.get_image() {
        // arboard::ImageData を PNG にエンコード
        if let Some(png_data) = encode_rgba_to_png(
            &img.bytes,
            img.width as u32,
            img.height as u32,
        ) {
            if let Ok(mut s) = store.lock() {
                s.add_image(&png_data);
            }
        }
    }
}

/// RGBA バイト列を PNG にエンコード（最小限の PNG エンコーダ）
///
/// 外部クレート不要で PNG を生成する。
/// パフォーマンスよりも依存関係の少なさを優先。
#[cfg(any(windows, test))]
fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    // 簡易 PNG エンコード: 非圧縮（deflate stored blocks）
    // 完全な PNG 仕様ではないが、主要ビューアで表示可能

    let w = width as usize;
    let h = height as usize;

    if rgba.len() < w * h * 4 {
        return None;
    }

    // フィルタバイト（0 = None）を各行の先頭に追加した生データ
    let mut raw_data = Vec::with_capacity(h * (1 + w * 4));
    for y in 0..h {
        raw_data.push(0u8); // filter: None
        let row_start = y * w * 4;
        let row_end = row_start + w * 4;
        raw_data.extend_from_slice(&rgba[row_start..row_end]);
    }

    // deflate (stored, non-compressed)
    let deflated = deflate_stored(&raw_data);

    // IDAT chunk
    let idat = make_chunk(b"IDAT", &deflated);

    // IHDR
    let mut ihdr_data = Vec::with_capacity(13);
    ihdr_data.extend_from_slice(&width.to_be_bytes());
    ihdr_data.extend_from_slice(&height.to_be_bytes());
    ihdr_data.push(8); // bit depth
    ihdr_data.push(6); // color type: RGBA
    ihdr_data.push(0); // compression
    ihdr_data.push(0); // filter
    ihdr_data.push(0); // interlace
    let ihdr = make_chunk(b"IHDR", &ihdr_data);

    let iend = make_chunk(b"IEND", &[]);

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]); // PNG signature
    png.extend_from_slice(&ihdr);
    png.extend_from_slice(&idat);
    png.extend_from_slice(&iend);

    Some(png)
}

#[cfg(any(windows, test))]
fn make_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let length = data.len() as u32;
    let mut chunk = Vec::with_capacity(12 + data.len());
    chunk.extend_from_slice(&length.to_be_bytes());
    chunk.extend_from_slice(chunk_type);
    chunk.extend_from_slice(data);

    // CRC32 over type + data
    let mut crc_data = Vec::with_capacity(4 + data.len());
    crc_data.extend_from_slice(chunk_type);
    crc_data.extend_from_slice(data);
    let crc = crc32(&crc_data);
    chunk.extend_from_slice(&crc.to_be_bytes());

    chunk
}

#[cfg(any(windows, test))]
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFFFFFF
}

#[cfg(any(windows, test))]
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    // zlib header + stored deflate blocks + adler32
    let mut out = Vec::new();
    out.push(0x78); // CMF: deflate, window size 32K
    out.push(0x01); // FLG: no dict, check bits

    // Split into 65535-byte blocks (max for stored blocks)
    let max_block = 65535usize;
    let mut offset = 0;

    while offset < data.len() {
        let remaining = data.len() - offset;
        let block_size = remaining.min(max_block);
        let is_last = offset + block_size >= data.len();

        out.push(if is_last { 0x01 } else { 0x00 }); // BFINAL + BTYPE=00 (stored)
        let len = block_size as u16;
        let nlen = !len;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&data[offset..offset + block_size]);

        offset += block_size;
    }

    // Adler-32 checksum
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());

    out
}

#[cfg(any(windows, test))]
fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// 非Windows環境用スタブ
#[cfg(not(windows))]
fn run_clipboard_monitor(_store: SharedStore) {
    eprintln!("[clipboard_history] クリップボード監視は Windows でのみ動作します");
}

// =============================================================================
// テスト
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- CRC32 のテスト ---

    #[test]
    fn test_crc32_empty() {
        assert_eq!(crc32(&[]), 0x00000000);
    }

    #[test]
    fn test_crc32_known_value() {
        // "IEND" の CRC32 は既知の値: 0xAE426082
        let data = b"IEND";
        let crc = crc32(data);
        assert_eq!(crc, 0xAE426082);
    }

    #[test]
    fn test_crc32_hello() {
        // "Hello" の CRC32 = 0xF7D18982
        let crc = crc32(b"Hello");
        assert_eq!(crc, 0xF7D18982);
    }

    #[test]
    fn test_crc32_deterministic() {
        let data = b"test data for crc";
        assert_eq!(crc32(data), crc32(data));
    }

    // --- Adler32 のテスト ---

    #[test]
    fn test_adler32_empty() {
        // Adler-32 of empty data = 1
        assert_eq!(adler32(&[]), 0x00000001);
    }

    #[test]
    fn test_adler32_known_value() {
        // "Wikipedia" の Adler-32 = 0x11E60398
        let result = adler32(b"Wikipedia");
        assert_eq!(result, 0x11E60398);
    }

    #[test]
    fn test_adler32_deterministic() {
        let data = b"test data";
        assert_eq!(adler32(data), adler32(data));
    }

    // --- deflate_stored のテスト ---

    #[test]
    fn test_deflate_stored_basic() {
        let data = b"Hello";
        let result = deflate_stored(data);

        // zlib header
        assert_eq!(result[0], 0x78); // CMF
        assert_eq!(result[1], 0x01); // FLG

        // Last block marker (BFINAL=1, BTYPE=00)
        assert_eq!(result[2], 0x01);

        // Block length = 5
        assert_eq!(result[3], 5);
        assert_eq!(result[4], 0);

        // ~Block length
        assert_eq!(result[5], !5u8);
        assert_eq!(result[6], 0xFF);

        // Data
        assert_eq!(&result[7..12], b"Hello");

        // Adler-32 at end (4 bytes, big-endian)
        let adler = adler32(b"Hello");
        let adler_bytes = adler.to_be_bytes();
        assert_eq!(&result[12..16], &adler_bytes);
    }

    #[test]
    fn test_deflate_stored_large_data() {
        // 65535バイト超のデータは複数ブロックに分割される
        let data = vec![0xAB; 70000];
        let result = deflate_stored(&data);

        // zlib header
        assert_eq!(result[0], 0x78);
        assert_eq!(result[1], 0x01);

        // 最初のブロック: BFINAL=0 (not last)
        assert_eq!(result[2], 0x00);

        // 出力にデータが含まれている
        assert!(result.len() > 70000);
    }

    // --- make_chunk のテスト ---

    #[test]
    fn test_make_chunk_iend() {
        let chunk = make_chunk(b"IEND", &[]);

        // Length = 0
        assert_eq!(&chunk[0..4], &[0, 0, 0, 0]);
        // Type = "IEND"
        assert_eq!(&chunk[4..8], b"IEND");
        // CRC = crc32("IEND")
        let expected_crc = crc32(b"IEND").to_be_bytes();
        assert_eq!(&chunk[8..12], &expected_crc);
    }

    #[test]
    fn test_make_chunk_with_data() {
        let data = vec![1, 2, 3, 4, 5];
        let chunk = make_chunk(b"tESt", &data);

        // Length = 5
        assert_eq!(&chunk[0..4], &5u32.to_be_bytes());
        // Type
        assert_eq!(&chunk[4..8], b"tESt");
        // Data
        assert_eq!(&chunk[8..13], &[1, 2, 3, 4, 5]);
        // CRC (over type + data)
        let mut crc_input = Vec::new();
        crc_input.extend_from_slice(b"tESt");
        crc_input.extend_from_slice(&data);
        let expected_crc = crc32(&crc_input).to_be_bytes();
        assert_eq!(&chunk[13..17], &expected_crc);
    }

    // --- encode_rgba_to_png のテスト ---

    #[test]
    fn test_encode_rgba_to_png_valid() {
        // 2x2 の赤い画像
        let rgba: Vec<u8> = vec![
            255, 0, 0, 255, // pixel (0,0): red
            255, 0, 0, 255, // pixel (1,0): red
            255, 0, 0, 255, // pixel (0,1): red
            255, 0, 0, 255, // pixel (1,1): red
        ];
        let result = encode_rgba_to_png(&rgba, 2, 2);
        assert!(result.is_some());

        let png = result.unwrap();
        // PNG シグネチャの確認
        assert_eq!(&png[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn test_encode_rgba_to_png_1x1() {
        let rgba = vec![0, 128, 255, 255]; // 1ピクセル
        let result = encode_rgba_to_png(&rgba, 1, 1);
        assert!(result.is_some());
    }

    #[test]
    fn test_encode_rgba_to_png_insufficient_data() {
        // 2x2 画像なのにデータが足りない
        let rgba = vec![255, 0, 0]; // 3バイトしかない（最低16バイト必要）
        let result = encode_rgba_to_png(&rgba, 2, 2);
        assert!(result.is_none());
    }

    #[test]
    fn test_encode_rgba_to_png_contains_ihdr_idat_iend() {
        let rgba = vec![0u8; 4]; // 1x1
        let png = encode_rgba_to_png(&rgba, 1, 1).unwrap();
        // チャンクタイプが含まれているか確認
        assert!(png.windows(4).any(|w| w == b"IHDR"));
        assert!(png.windows(4).any(|w| w == b"IDAT"));
        assert!(png.windows(4).any(|w| w == b"IEND"));
    }

    // --- SharedStore のテスト ---

    #[test]
    fn test_shared_store_type() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ClipboardStore::new_with_dir(tmp.path().to_path_buf(), 7);
        let shared: SharedStore = Arc::new(Mutex::new(store));

        // 複数のクローンが同じストアを参照できる
        let clone1 = shared.clone();
        let clone2 = shared.clone();

        if let Ok(mut s) = clone1.lock() {
            s.add_text("shared entry");
        }

        if let Ok(s) = clone2.lock() {
            assert_eq!(s.len(), 1);
        };
    }
}

// notification.rs - Windows トースト通知
//
// PowerShell の System.Windows.Forms.NotifyIcon を使って
// バルーン通知を表示する。追加のクレート不要。
//
// 制約:
// - Windows 10 以降でのみ動作
// - PowerShell の起動に ~200ms かかるが、非同期なのでブロックしない

use std::process::Command;
use std::thread;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// PowerShell バルーン通知に安全な文字列に変換する
///
/// - シングルクォートをエスケープ
/// - 改行をスペースに
/// - 長すぎるメッセージを切り詰め（バルーン通知は約200文字が限界）
fn sanitize_for_balloon(s: &str, max_len: usize) -> String {
    let cleaned = s.replace('\'', "''").replace('\n', " ").replace('\r', " ");
    if cleaned.len() > max_len {
        format!("{}...", &cleaned[..max_len])
    } else {
        cleaned
    }
}

/// バルーン通知を表示する（非同期・ノンブロッキング）
///
/// PowerShell を裏で起動してシステム通知を出す。
/// 失敗しても無視する（通知は必須機能ではないため）。
#[cfg(windows)]
pub fn show(title: &str, message: &str) {
    let title = sanitize_for_balloon(title, 60);
    let message = sanitize_for_balloon(message, 200);

    let script = format!(
        r#"Add-Type -AssemblyName System.Windows.Forms
$n = New-Object System.Windows.Forms.NotifyIcon
$n.Icon = [System.Drawing.SystemIcons]::Information
$n.Visible = $true
$n.ShowBalloonTip(3000, '{}', '{}', 'Info')
Start-Sleep -Seconds 4
$n.Dispose()"#,
        title, message
    );

    thread::spawn(move || {
        let _ = Command::new("powershell")
            .args([
                "-WindowStyle", "Hidden",
                "-ExecutionPolicy", "Bypass",
                "-Command", &script,
            ])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
    });
}

#[cfg(not(windows))]
pub fn show(title: &str, message: &str) {
    eprintln!("[notification] {}: {}", title, message);
}

/// エラー通知を表示する
pub fn show_error(title: &str, message: &str) {
    #[cfg(windows)]
    {
        let title = sanitize_for_balloon(title, 60);
        let message = sanitize_for_balloon(message, 200);

        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
$n = New-Object System.Windows.Forms.NotifyIcon
$n.Icon = [System.Drawing.SystemIcons]::Warning
$n.Visible = $true
$n.ShowBalloonTip(5000, '{}', '{}', 'Warning')
Start-Sleep -Seconds 6
$n.Dispose()"#,
            title, message
        );

        thread::spawn(move || {
            let _ = Command::new("powershell")
                .args([
                    "-WindowStyle", "Hidden",
                    "-ExecutionPolicy", "Bypass",
                    "-Command", &script,
                ])
                .creation_flags(0x08000000)
                .spawn();
        });
    }

    #[cfg(not(windows))]
    {
        eprintln!("[notification error] {}: {}", title, message);
    }
}

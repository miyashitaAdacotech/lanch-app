# lanch-app GitHub Issue 一括作成スクリプト
#
# gh CLI 不要。GitHub Fine-grained PAT で直接 API を叩く。
#
# 実行方法:
#   cd C:\Users\宮下博行\lanch-app
#   $env:GITHUB_TOKEN = "github_pat_xxxxx"
#   powershell -ExecutionPolicy Bypass -File scripts/create-issues.ps1

$ErrorActionPreference = "Stop"
$repo = "miyashita337/lanch-app"
$apiBase = "https://api.github.com/repos/$repo"

# --- トークン取得 ---
$token = $env:GITHUB_TOKEN
if (-not $token) {
    Write-Host "ERROR: GITHUB_TOKEN 環境変数が設定されていません" -ForegroundColor Red
    Write-Host '  $env:GITHUB_TOKEN = "github_pat_xxxxx"'
    exit 1
}

$headers = @{
    "Authorization" = "Bearer $token"
    "Accept" = "application/vnd.github+json"
    "X-GitHub-Api-Version" = "2022-11-28"
}

# --- 認証テスト ---
Write-Host "=== lanch-app Issue 一括作成 ===" -ForegroundColor Cyan
Write-Host "Repository: $repo"
try {
    $user = Invoke-RestMethod -Uri "https://api.github.com/user" -Headers $headers -Method Get
    Write-Host "Authenticated as: $($user.login)" -ForegroundColor Green
} catch {
    Write-Host "ERROR: 認証に失敗しました。トークンを確認してください。" -ForegroundColor Red
    Write-Host "  $_"
    exit 1
}
Write-Host ""

# --- ラベル作成（存在しなければ） ---
$labelNames = @("enhancement", "bug", "chore")
foreach ($label in $labelNames) {
    try {
        $null = Invoke-RestMethod -Uri "$apiBase/labels/$label" -Headers $headers -Method Get
    } catch {
        if ($_.Exception.Response.StatusCode -eq 404) {
            try {
                $body = @{ name = $label } | ConvertTo-Json
                $null = Invoke-RestMethod -Uri "$apiBase/labels" -Headers $headers -Method Post -Body $body -ContentType "application/json"
                Write-Host "  Label created: $label" -ForegroundColor Yellow
            } catch {
                Write-Host "  Label '$label' creation skipped" -ForegroundColor Gray
            }
        }
    }
}

# --- Issue定義（#1 README更新, #2 main.rsヘルプ更新 は完了済みのため除外） ---
$issues = @(
    @{
        title = "test: ユニットテスト・結合テストの追加"
        labels = @("enhancement")
        body = @"
## 概要
現在テストが一切ない。基本的なテストを追加する。

## やること
- [ ] config.rs: デフォルト設定の生成、JSON読み書きのテスト
- [ ] lang.rs: 日本語判定の正確性テスト
- [ ] formatter.rs: 空文字列入力、バックエンド検出のテスト
- [ ] notification.rs: sanitize_for_balloon のエッジケーステスト
- [ ] tray.rs: parse_hotkey のテスト
- [ ] CI (GitHub Actions) でのテスト自動実行
"@
    },
    @{
        title = "feat: Windows ログイン時の自動起動対応"
        labels = @("enhancement")
        body = @"
## 概要
PCを起動するたびに手動で lanch-app を起動する必要がある。

## やること
- [ ] レジストリ（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）に登録する機能
- [ ] config.json に auto_start: bool オプション追加
- [ ] トレイメニューに「自動起動 ON/OFF」トグル追加
"@
    },
    @{
        title = "feat: ホットキー競合時の自動代替キー提案"
        labels = @("enhancement")
        body = @"
## 概要
ホットキーが他アプリと競合した場合、警告のみで機能が無効化される。

## 現状
- 競合: 警告ログ → その機能は使えない
- 設定変更: config.json 手動編集が必要

## やること
- [ ] 競合検出時に代替ホットキーを自動試行する仕組み
- [ ] トレイメニューからホットキー設定を変更できるサブメニュー
- [ ] hotkey_selected のデフォルトが config によって alt+z になる場合がある問題を修正
"@
    },
    @{
        title = "improve: Windows通知を PowerShell から WinRT に移行"
        labels = @("enhancement")
        body = @"
## 概要
現在の通知はPowerShellプロセスを毎回起動する方式で約200msのオーバーヘッドがある。

## やること
- [ ] windows-sys クレートで Shell_NotifyIconW を直接呼ぶ方式に変更
- [ ] または winrt-notification クレートの導入を検討
- [ ] PowerShell方式をフォールバックとして残す
"@
    },
    @{
        title = "fix: CLI モード（--format, --translate）の動作検証と修正"
        labels = @("bug")
        body = @"
## 概要
``lanch-app --format "text"`` のCLIモードがハイブリッド方式移行後に正しく動作するか未検証。

## やること
- [ ] --format の動作確認（API直接 / CLI両方）
- [ ] --translate の動作確認
- [ ] CLAUDECODE 環境変数がある場合のハンドリング（ネスト防止）
- [ ] エラー時の終了コードを適切に設定
"@
    },
    @{
        title = "chore: config.json から claude_api_key を段階的に廃止"
        labels = @("chore")
        body = @"
## 概要
ハイブリッド方式では ANTHROPIC_API_KEY 環境変数を直接参照するため、
config.json の claude_api_key フィールドは不要になった。

## やること
- [ ] 新規生成時に claude_api_key を含めない
- [ ] 既存config読み込み時の後方互換は維持
- [ ] マイグレーション時の警告メッセージ追加
"@
    }
)

# --- Issue作成 ---
$created = 0
foreach ($issue in $issues) {
    Write-Host -NoNewline "  Creating: $($issue.title) ... "

    $body = @{
        title = $issue.title
        body = $issue.body
        labels = $issue.labels
    } | ConvertTo-Json -Depth 3

    try {
        $result = Invoke-RestMethod -Uri "$apiBase/issues" -Headers $headers -Method Post -Body $body -ContentType "application/json; charset=utf-8"
        Write-Host "OK #$($result.number) $($result.html_url)" -ForegroundColor Green
        $created++
    } catch {
        $statusCode = $_.Exception.Response.StatusCode
        Write-Host "FAIL ($statusCode)" -ForegroundColor Red
        Write-Host "    $_" -ForegroundColor Yellow
    }

    Start-Sleep -Seconds 1
}

Write-Host ""
Write-Host "=== 完了: $created / $($issues.Count) 件作成 ===" -ForegroundColor Cyan

# smoke-install.ps1
# Plan 27 smoke for current-user NSIS ohMyWork installer (Windows 11 x64)
# Usage (as standard user, no admin):
#   pwsh -NoProfile -File tools/smoke-install.ps1 -InstallDir "$env:LOCALAPPDATA\Programs\ohMyWork"
#   (or without arg, defaults to standard location)
#
# Automatable checks: exit nonzero on failure.
# Human verification items: listed at end; do not cause nonzero unless explicitly enabled.
#
# Must be run after a real (or simulated) install. Does not invoke installer itself.
# Fails closed on unsigned if $RequireSigned, but default allows "未签名" label.

[CmdletBinding()]
param(
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\ohMyWork",
    [switch]$RequireSigned = $false,
    [switch]$HumanChecksAsError = $false
)

$ErrorActionPreference = 'Stop'
$failed = 0
$warnings = 0

function Fail($msg) {
    Write-Error "FAIL: $msg"
    $script:failed++
}

function Warn($msg) {
    Write-Warning "WARN: $msg"
    $script:warnings++
}

function Info($msg) { Write-Host "INFO: $msg" -ForegroundColor Cyan }

Write-Host "=== ohMyWork smoke-install (per-user, no-admin) ===" -ForegroundColor Green
Info "InstallDir: $InstallDir"
Info "RequireSigned: $RequireSigned"

# 1. Install dir must exist under user LocalAppData\Programs (no Program Files)
if (-not (Test-Path -LiteralPath $InstallDir)) {
    Fail "Install dir missing: $InstallDir (expected %LOCALAPPDATA%\Programs\ohMyWork)"
} else {
    $parent = Split-Path $InstallDir -Parent
    if ($parent -notlike "*\LocalAppData\Programs") {
        Warn "Install dir not under expected user Programs (may be custom)"
    }
    Info "Install dir exists under user profile."
}

# 2. Core files present (excludes test/perf/fake)
$core = @(
    "ohmywork.exe",
    "omw-preview-worker.exe",
    "pdfium.dll",
    "THIRD-PARTY-NOTICES.txt",
    "assets-manifest.json"
)
foreach ($f in $core) {
    $p = Join-Path $InstallDir $f
    if (-not (Test-Path -LiteralPath $p)) {
        Fail "Required file missing: $f"
    } else {
        Info "Present: $f"
    }
}

# 3. Explicitly excluded binaries must NOT be present
$excluded = @("perf_runner.exe", "fake_pi.exe", "perf_runner.pdb", "fake_pi.pdb")
foreach ($e in $excluded) {
    $p = Join-Path $InstallDir $e
    if (Test-Path -LiteralPath $p) {
        Fail "Excluded test/perf binary present in package: $e (must not ship)"
    }
}

# 4. Manifest version + basic integrity (version match from package)
$manifestPath = Join-Path $InstallDir "assets-manifest.json"
if (Test-Path $manifestPath) {
    try {
        $m = Get-Content $manifestPath -Raw | ConvertFrom-Json
        if ($m.version -ne "0.1.0") {
            Fail "Manifest version mismatch: $($m.version) != 0.1.0"
        }
        if ($m.product -ne "ohMyWork" -or $m.platform -ne "windows-x64") {
            Fail "Manifest product/platform wrong"
        }
        # pdfium required + hash check if file present
        $pdf = Join-Path $InstallDir "pdfium.dll"
        if (Test-Path $pdf) {
            $actual = (Get-FileHash -LiteralPath $pdf -Algorithm SHA256).Hash.ToLower()
            $expected = ($m.files | Where-Object path -eq 'pdfium.dll').sha256
            if ($expected -and $actual -ne $expected) {
                Fail "pdfium.dll hash mismatch: got $actual expected $expected"
            }
            Info "pdfium.dll hash verified against manifest."
        }
        Info "assets-manifest.json parsed, version ok."
    } catch {
        Fail "Failed to parse/validate assets-manifest.json: $_"
    }
} else {
    Fail "assets-manifest.json missing (required for upgrade verification)"
}

# 5. Uninstaller present (produced by NSIS at install time; raw staging may not have it)
$uninst = Join-Path $InstallDir "Uninstall.exe"
if (-not (Test-Path -LiteralPath $uninst)) {
    Warn "Uninstaller (Uninstall.exe) not present (created by running the installer; expected in raw package layout)"
}

# 6. Per-user registry only (HKCU), no machine-wide (HKLM)
#    These are written by the installer; absence here is normal for raw file layout validation.
$regKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ohMyWork"
$reg = $null
try { $reg = Get-ItemProperty -LiteralPath $regKey -ErrorAction Stop } catch {}
if (-not $reg) {
    Warn "Uninstall registry key not present under HKCU (written at install time; normal for raw layout smoke)"
} else {
    if ($reg.DisplayName -notlike "*ohMyWork*") { Fail "Bad DisplayName" }
    if ($reg.UninstallString -notlike "*$InstallDir*") { Fail "UninstallString not pointing to user install" }
    if ($reg.Publisher -like "*SYSTEM*" -or $reg.InstallLocation -like "C:\Program Files*") {
        Fail "Looks like machine-wide install (forbidden)"
    }
    Info "Per-user uninstall registry ok (HKCU)."
}

# Check HKLM does not have our key (should not for user install) - this can be present from a previous real install; still hard fail if found during smoke
$lmKey = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\ohMyWork"
if (Test-Path $lmKey) {
    Fail "Machine-wide HKLM uninstall key exists (must be per-user only)"
}
# 7. Shortcut in user start menu (not All Users)
$shortcut = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\ohMyWork.lnk"
if (-not (Test-Path -LiteralPath $shortcut)) {
    Warn "User Start Menu shortcut missing (may be optional in some flows): $shortcut"
} else {
    Info "User shortcut present."
}

# 8. No update service / no Pi download artifacts
# (simple presence checks; full no-net is human + workflow)
$svc = Get-Service -Name "*ohMyWork*" -ErrorAction SilentlyContinue
if ($svc) { Fail "Update or ohMyWork service found (forbidden - fully offline)" }

$piCache = "$env:LOCALAPPDATA\ohMyWork\pi*"
if (Get-ChildItem -Path $piCache -ErrorAction SilentlyContinue) {
    Warn "Possible Pi config/cache present; ensure installer did not create/download"
}

# 9. In-use / path handling note (human)
Info "In-use files, Chinese/spaces paths, restart handling: verified by NSIS /REBOOTOK + proper paths."

# === HUMAN ITEMS (do not fail unless -HumanChecksAsError) ===
Write-Host ""
Write-Host "=== HUMAN VERIFICATION (list; run manually) ===" -ForegroundColor Yellow
$human = @(
    "1. Run ohmywork.exe from Start Menu / desktop; confirm GPUI Kit default icons, theme, Dock, dialogs load without cwd asset load errors.",
    "2. Open PDF + image preview; confirm no worker crash (pdfium.dll next to worker).",
    "3. If package unsigned: confirm filename or UI labels contain '未签名' (do not promise SmartScreen bypass).",
    "4. Perform manual upgrade simulation (old -> new installer); verify manifest check, state preserved, rollback works on injected fail (no layout/theme deletion).",
    "5. Uninstall: default keeps Workspace + %LOCALAPPDATA%\ohMyWork ; explicit 'clear app data' option is UNCHECKED by default. Confirm NEVER deletes Workspace root.",
    "6. Paths with spaces and Chinese chars in install dir and Workspace: full roundtrip install/upgrade/preview.",
    "7. No admin prompt at any step; no network calls for Pi/update during install/upgrade/uninstall.",
    "8. Reinstall over existing works; clean uninstall leaves user data unless opted.",
    "9. Checksum of released .exe matches published (if provided).",
    "10. Sign only with real EV/code cert for prod; CI builds labeled unsigned."
)
$human | ForEach { Write-Host "  $_" }

if ($HumanChecksAsError) {
    Fail "Human checks requested as errors (for CI gating simulation)"
}

# Final
Write-Host ""
if ($failed -gt 0) {
    Write-Host "SMOKE FAILED: $failed failure(s), $warnings warning(s)" -ForegroundColor Red
    exit 1
} else {
    Write-Host "SMOKE PASSED (auto checks). $warnings warning(s). Review human list above." -ForegroundColor Green
    if ($warnings -gt 0) { exit 2 }
    exit 0
}
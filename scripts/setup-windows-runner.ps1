#requires -RunAsAdministrator
<#
.SYNOPSIS
    One-shot setup for the self-hosted GitHub Actions runner on the Hardwave
    DAW Windows build box (DESKTOP-R3EHHM9).

.DESCRIPTION
    Installs every tool the .github/workflows/release.yml + dev-build.yml
    Windows leg expects to find on PATH, then restarts the runner service
    so NETWORK SERVICE picks up the new System PATH entries.

    Run once after registering the runner (or after a Windows reinstall).
    Idempotent — safe to re-run; winget skips packages that are up to date.

.NOTES
    The runner service runs as NT AUTHORITY\NETWORK SERVICE which only
    sees the System PATH, never the User PATH. Every install in this
    script uses winget's default machine-wide scope so it lands on the
    System PATH.

    The runner service name embeds the repo owner/name + hostname:
        actions.runner.Dishairano-hardwave-daw.DESKTOP-R3EHHM9
    Adjust SERVICE_NAME below if you register the runner against a
    different repo or hostname.

.EXAMPLE
    PS> .\scripts\setup-windows-runner.ps1
#>

[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

$SERVICE_NAME = 'actions.runner.Dishairano-hardwave-daw.DESKTOP-R3EHHM9'

function Section($msg) {
    Write-Host ''
    Write-Host "==> $msg" -ForegroundColor Cyan
}

function WingetInstall($id, $label) {
    Section "Installing $label ($id)"
    # --accept-package-agreements + --accept-source-agreements skips the
    # interactive Y/N prompts so the script can run unattended.
    winget install --id $id --source winget `
        --accept-package-agreements --accept-source-agreements `
        --silent | Out-Host
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne -1978335189) {
        # -1978335189 = APPINSTALLER_CLI_ERROR_UPDATE_NOT_APPLICABLE (already up to date)
        Write-Warning "winget returned exit code $LASTEXITCODE for $id (continuing)"
    }
}

# ---- 0. sanity check ----
Section 'Confirming admin context'
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator
)
if (-not $isAdmin) {
    Write-Error 'This script must run in an elevated PowerShell session (Run as Administrator).'
    exit 1
}
Write-Host 'OK — running as Administrator.'

# ---- 1. Git for Windows (provides bash.exe — required by dtolnay/rust-toolchain) ----
WingetInstall 'Git.Git' 'Git for Windows'

# ---- 2. Rust toolchain (rustup + cargo + stable, machine-wide) ----
WingetInstall 'Rustlang.Rustup' 'Rust (rustup-init)'

# Configure the stable toolchain + the MSVC target the workflows build.
# Run in a fresh shell so the rustup-just-installed PATH update is seen.
Section 'Configuring Rust default + msvc target'
$rustupPath = "$env:USERPROFILE\.cargo\bin\rustup.exe"
if (-not (Test-Path $rustupPath)) {
    # winget put rustup in C:\Users\<admin>\.cargo\bin which NETWORK
    # SERVICE will NOT see. Hint: also installs at
    # C:\Program Files\Rust stable MSVC 1.x.x — but that's user-local
    # too. Easiest path: let `dtolnay/rust-toolchain@stable` continue
    # to install Rust into the runner's tool cache (machine-readable
    # location). Skip the rustup default step here; just leave the
    # binary present for local sanity-check use.
    Write-Warning "rustup not yet on PATH for this admin session — workflow's dtolnay action will install Rust into tool cache instead. OK."
} else {
    & $rustupPath default stable | Out-Host
    & $rustupPath target add x86_64-pc-windows-msvc | Out-Host
}

# ---- 3. LLVM (libclang for cpal/asio bindgen + lld-link for fast linking) ----
WingetInstall 'LLVM.LLVM' 'LLVM (clang + lld)'

# ---- 4. Node.js LTS (workflow uses actions/setup-node@v4 too, but having
#         Node machine-wide means the runner can call `npm install -g
#         @tauri-apps/cli` without re-downloading every run) ----
WingetInstall 'OpenJS.NodeJS.LTS' 'Node.js LTS'

# ---- 5. Verify bash is reachable (the failing step) ----
Section 'Verifying bash.exe (Git Bash) is on System PATH before the WSL launcher'
$gitBash = 'C:\Program Files\Git\bin\bash.exe'
if (Test-Path $gitBash) {
    $out = & $gitBash -c 'echo ok'
    if ($out -eq 'ok') {
        Write-Host "Git Bash works: $gitBash -> '$out'"
    } else {
        Write-Warning "Git Bash returned unexpected output: '$out'"
    }
} else {
    Write-Warning "$gitBash not found — check Git for Windows install"
}

# ---- 6. Verify LLVM paths the workflow needs ----
Section 'Verifying LLVM libclang.dll + lld-link.exe'
$llvmBin = 'C:\Program Files\LLVM\bin'
foreach ($file in @('libclang.dll', 'lld-link.exe', 'clang.exe')) {
    $p = Join-Path $llvmBin $file
    if (Test-Path $p) {
        Write-Host "  found: $p"
    } else {
        Write-Warning "  MISSING: $p"
    }
}

# ---- 7. Restart the runner service so NETWORK SERVICE picks up the new System PATH ----
Section "Restarting runner service: $SERVICE_NAME"
$svc = Get-Service -Name $SERVICE_NAME -ErrorAction SilentlyContinue
if (-not $svc) {
    Write-Warning "Service '$SERVICE_NAME' not found — did you register the runner under this exact hostname/repo?"
    Write-Warning 'Listing runner services that ARE installed:'
    Get-Service -Name 'actions.runner.*' | Format-Table -AutoSize | Out-Host
} else {
    if ($svc.Status -eq 'Running') {
        sc.exe stop $SERVICE_NAME | Out-Host
        # Give it a moment to fully release file handles.
        Start-Sleep -Seconds 3
    }
    sc.exe start $SERVICE_NAME | Out-Host
    Start-Sleep -Seconds 2
    $svc.Refresh()
    Write-Host "Service status after restart: $($svc.Status)"
}

# ---- 8. Summary ----
Section 'Done — verify on GitHub that the runner is online'
Write-Host @"
Next steps:
  1. Open https://github.com/Dishairano/hardwave-daw/settings/actions/runners
     The runner should show 'Idle' (green dot). If 'Offline', the service didn't restart cleanly.
  2. Re-run the failing Dev Build:
     https://github.com/Dishairano/hardwave-daw/actions/workflows/dev-build.yml
     Re-run failed jobs — only the Windows leg will fire on this runner.
  3. Watch live logs:
     Get-Content 'C:\actions-runner\_diag\Runner_*.log' -Tail 50 -Wait

Future optimization (manual, one-time):
  - Download Steinberg ASIO SDK from https://www.steinberg.net/asiosdk
  - Extract to e.g. C:\sdks\asiosdk_2.3.3
  - System Properties → Environment Variables → New System variable:
      Name:  CPAL_ASIO_DIR
      Value: C:\sdks\asiosdk_2.3.3
  - Restart the runner service again so it sees the new var.
  - Saves ~30s per build (ASIO SDK download skipped).
"@

# Make sure cmake is on PATH for this job.
#
# Ableton Link builds its C library with cmake. Installing cmake with
# winget puts it on the machine's PATH, but a service that was already
# running keeps the environment it started with, so the Actions runner
# goes on saying "cmake is not recognized" until someone restarts it.
# Rather than ask for that, this looks in the places winget and the
# installer put it and adds the one it finds to this job's PATH.
$ErrorActionPreference = 'Stop'

if (Get-Command cmake -ErrorAction SilentlyContinue) {
    cmake --version
    exit 0
}

$candidates = @(
    (Join-Path $env:ProgramFiles 'CMake\bin'),
    (Join-Path ${env:ProgramFiles(x86)} 'CMake\bin'),
    (Join-Path $env:LOCALAPPDATA 'Programs\CMake\bin'),
    (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links')
)
foreach ($dir in $candidates) {
    if (-not $dir) { continue }
    if (Test-Path (Join-Path $dir 'cmake.exe')) {
        Write-Host "found cmake in $dir, adding it to this job's PATH"
        Add-Content -Path $env:GITHUB_PATH -Value $dir
        & (Join-Path $dir 'cmake.exe') --version
        exit 0
    }
}

Write-Host 'cmake is missing. Install it once with winget install Kitware.CMake'
Write-Host 'If it is installed, restart the Actions runner service so it picks up the new PATH.'
exit 1

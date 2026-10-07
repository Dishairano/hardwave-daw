# Copy the test installer the build just made to C:\Hardwave Test Builds,
# once with the date and once as "(latest)", so the founder finds it in
# the same place every time.
$ErrorActionPreference = 'Stop'

$dir = 'C:\Hardwave Test Builds'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$candidates = @(
    'target\x86_64-pc-windows-msvc\release\bundle\nsis',
    'src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis'
) | Where-Object { Test-Path $_ }
$exe = Get-ChildItem -Path $candidates -Filter '*_x64-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $exe) {
    throw 'no installer was built'
}
$name = 'Hardwave DAW test ' + (Get-Date -Format 'yyyy-MM-dd HHmm') + '.exe'
Copy-Item $exe.FullName (Join-Path $dir $name)
Copy-Item $exe.FullName (Join-Path $dir 'Hardwave DAW test (latest).exe') -Force
Write-Host "Ready: $(Join-Path $dir $name)"

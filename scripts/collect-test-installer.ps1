# Copy the test installer the build just made to C:\Hardwave Test Builds,
# once with the date and once as "(latest)", so the founder finds it in
# the same place every time.
$ErrorActionPreference = 'Stop'

# The laptop's runner says where through its environment; the PC keeps
# C:\Hardwave Test Builds.
$dir = if ($env:HW_TEST_BUILDS_DIR) { $env:HW_TEST_BUILDS_DIR } else { 'C:\Hardwave Test Builds' }
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$candidates = @(
    'target\x86_64-pc-windows-msvc\release\bundle\nsis',
    'src-tauri\target\x86_64-pc-windows-msvc\release\bundle\nsis'
)
# A build folder set outside the checkout (the laptop keeps paths short).
if ($env:CARGO_TARGET_DIR) {
    $candidates += Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc\release\bundle\nsis'
}
$candidates = $candidates | Where-Object { Test-Path $_ }
$exe = Get-ChildItem -Path $candidates -Filter '*_x64-setup.exe' |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $exe) {
    throw 'no installer was built'
}
$name = 'Hardwave DAW test ' + (Get-Date -Format 'yyyy-MM-dd HHmm') + '.exe'
Copy-Item $exe.FullName (Join-Path $dir $name)
Copy-Item $exe.FullName (Join-Path $dir 'Hardwave DAW test (latest).exe') -Force
Write-Host "Ready: $(Join-Path $dir $name)"

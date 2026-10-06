# Make sure the native build tools this workspace needs are usable
# in this job: cmake, and libclang for bindgen.
#
# Ableton Link builds its C library with cmake. Installing cmake with
# winget puts it on the machine's PATH, but a service that was already
# running keeps the environment it started with, so the Actions runner
# goes on saying "cmake is not recognized" until someone restarts it.
# Rather than ask for that, this looks in the places winget and the
# installer put it and adds the one it finds to this job's PATH.
$ErrorActionPreference = 'Stop'

function Find-Dir($paths, $file) {
    foreach ($dir in $paths) {
        if (-not $dir) { continue }
        if (Test-Path (Join-Path $dir $file)) { return $dir }
    }
    return $null
}

# cmake, for Ableton Link's C library.
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    $cmakeDir = Find-Dir @(
        (Join-Path $env:ProgramFiles 'CMake\bin'),
        (Join-Path ${env:ProgramFiles(x86)} 'CMake\bin'),
        (Join-Path $env:LOCALAPPDATA 'Programs\CMake\bin'),
        (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links')
    ) 'cmake.exe'
    if (-not $cmakeDir) {
        Write-Host 'cmake is missing. Install it once with   winget install Kitware.CMake'
        exit 1
    }
    Write-Host "found cmake in $cmakeDir, adding it to this job's PATH"
    Add-Content -Path $env:GITHUB_PATH -Value $cmakeDir
}

# libclang, which bindgen loads to read the C headers. Ableton Link
# and the ASIO bindings both go through it.
if (-not $env:LIBCLANG_PATH) {
    $clangDir = Find-Dir @(
        (Join-Path $env:ProgramFiles 'LLVM\bin'),
        (Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin'),
        (Join-Path $env:LOCALAPPDATA 'Programs\LLVM\bin')
    ) 'libclang.dll'
    if (-not $clangDir) {
        Write-Host 'libclang is missing. Install LLVM once with   winget install LLVM.LLVM'
        Write-Host 'bindgen needs it to read the C headers for Ableton Link.'
        exit 1
    }
    Write-Host "found libclang in $clangDir"
    Add-Content -Path $env:GITHUB_ENV -Value "LIBCLANG_PATH=$clangDir"
    Add-Content -Path $env:GITHUB_PATH -Value $clangDir
}

Write-Host 'build tools are in place'
exit 0

# Install the test build that was just made, on this machine, for the
# person (or the agent) testing it here. Per-user, no administrator, the
# same silent install the Hardwave link does. Only for laptop-test-install-*
# tags, so an ordinary test build never replaces what is installed.
#
# An open DAW is left alone: installing over it would close it without
# asking, and whatever was not saved would be lost.
$ErrorActionPreference = 'Stop'
$dir = if ($env:HW_TEST_BUILDS_DIR) { $env:HW_TEST_BUILDS_DIR } else { 'C:\Hardwave Test Builds' }
$setup = Join-Path $dir 'Hardwave DAW test (latest).exe'
if (-not (Test-Path $setup)) { throw "no test installer at $setup" }

$installDirs = @((Join-Path $env:LOCALAPPDATA 'Hardwave DAW'), (Join-Path $env:LOCALAPPDATA 'Programs\Hardwave DAW'))
$running = Get-Process -ErrorAction SilentlyContinue | Where-Object {
  $p = $_.Path
  $p -and ($installDirs | Where-Object { $p.StartsWith($_, [StringComparison]::OrdinalIgnoreCase) })
}
if ($running) {
  Write-Host "The DAW is open on this machine, so the test build was not installed (it would close it). Close it and run the install again."
  exit 0
}

Unblock-File $setup -ErrorAction SilentlyContinue
$p = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "the installer stopped with code $($p.ExitCode)" }
foreach ($d in $installDirs) {
  $exe = Get-ChildItem -Path $d -Filter '*.exe' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notmatch 'uninstall' } | Select-Object -First 1
  if ($exe) { Write-Host "Installed: $($exe.FullName)"; exit 0 }
}
throw 'the installer finished but the DAW is not where it should be'

# The ASIO SDK, fetched once into the runner's tool cache and kept.
#
# Used only when it is the exact file whose hash is recorded here: the
# SDK is compiled into the DAW, so a changed download must not slip in.
# Writes CPAL_ASIO_DIR for the steps after it.
$ErrorActionPreference = 'Stop'

$version = '2.3.4'
$url = 'https://download.steinberg.net/sdk_downloads/ASIO-SDK_2.3.4_2025-10-15.zip'
$sha256 = 'D5EBF0C20DD2C5F43771FD0C1418F4B361BF52434EE670097CFA6B3A335E2ECA'

$root = Join-Path $env:RUNNER_TOOL_CACHE "asiosdk-$version"
$sdk = Join-Path $root 'ASIOSDK'
if (-not (Test-Path $sdk)) {
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    $zip = Join-Path $env:RUNNER_TEMP 'asiosdk.zip'
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
    $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash
    if ($actual -ne $sha256) {
        throw "the ASIO SDK download is not the expected file ($actual)"
    }
    Expand-Archive -Path $zip -DestinationPath $root -Force
}
"CPAL_ASIO_DIR=$sdk" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
Write-Host "ASIO SDK at $sdk"

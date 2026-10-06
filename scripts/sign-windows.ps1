# Sign a Windows binary with Azure Trusted Signing.
#
# Tauri calls this once per file it bundles, with the file as the only
# argument, so the installer and everything inside it are signed before
# the updater signature is taken over them. Signing afterwards would
# change the bytes the updater had already signed, and every running
# copy would refuse the update.
#
# Without the Azure settings in the environment this does nothing and
# says so: a local build has no business needing the company's
# certificate, and a developer's build should not fail for the want of
# one.
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string] $File
)

$ErrorActionPreference = 'Stop'

$endpoint = $env:AZURE_TRUSTED_SIGNING_ENDPOINT
$account  = $env:AZURE_TRUSTED_SIGNING_ACCOUNT
$profile  = $env:AZURE_TRUSTED_SIGNING_PROFILE

if (-not $endpoint -or -not $account -or -not $profile) {
    Write-Host "sign-windows: no Azure Trusted Signing settings, leaving $File unsigned"
    exit 0
}
if (-not $env:AZURE_TENANT_ID -or -not $env:AZURE_CLIENT_ID -or -not $env:AZURE_CLIENT_SECRET) {
    Write-Host "sign-windows: no Azure credentials, leaving $File unsigned"
    exit 0
}

# The client library signtool loads to talk to Trusted Signing. Cached
# under the runner's temp folder, so a build signs many files and
# downloads once.
$clientVersion = '1.0.86'
$clientRoot = Join-Path $env:RUNNER_TEMP "trusted-signing-$clientVersion"
$dlib = Join-Path $clientRoot 'bin/x64/Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path $dlib)) {
    New-Item -ItemType Directory -Force -Path $clientRoot | Out-Null
    $nupkg = Join-Path $env:RUNNER_TEMP 'trusted-signing-client.zip'
    $url = "https://www.nuget.org/api/v2/package/Microsoft.Trusted.Signing.Client/$clientVersion"
    Invoke-WebRequest -Uri $url -OutFile $nupkg -UseBasicParsing
    Expand-Archive -Path $nupkg -DestinationPath $clientRoot -Force
}
if (-not (Test-Path $dlib)) {
    throw "sign-windows: the Trusted Signing client is missing at $dlib"
}

$metadata = Join-Path $clientRoot 'metadata.json'
@{
    Endpoint               = $endpoint
    CodeSigningAccountName = $account
    CertificateProfileName = $profile
} | ConvertTo-Json | Set-Content -Path $metadata -Encoding utf8

# signtool from the Windows SDK, whichever version the runner has.
$signtool = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' `
    -ErrorAction SilentlyContinue | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $signtool) {
    throw 'sign-windows: signtool.exe was not found; the Windows SDK is not installed'
}

& $signtool.FullName sign `
    /v /fd SHA256 /tr 'http://timestamp.acs.microsoft.com' /td SHA256 `
    /dlib $dlib /dmdf $metadata `
    $File
if ($LASTEXITCODE -ne 0) {
    throw "sign-windows: signing $File failed with $LASTEXITCODE"
}
Write-Host "sign-windows: signed $File"

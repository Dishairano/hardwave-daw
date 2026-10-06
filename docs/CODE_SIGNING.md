# Signing the Windows build

The DAW is signed with Azure Trusted Signing, which signs with a
certificate Microsoft holds rather than one we keep in a file. There
is nothing to store, nothing to expire on a laptop, and nothing to
lose.

## What the build does

`src-tauri/tauri.conf.json` tells Tauri to run `scripts/sign-windows.ps1`
on every file it bundles. That happens before the updater signature is
taken, which matters: signing afterwards would change the bytes the
updater had already signed, and every running copy would refuse the
update.

Without the Azure settings the script says so and leaves the file
alone, so a local build, a fork and a dry run all still finish.

## What has to be set, once

In the repository's settings, under Secrets and variables > Actions:

**Secrets** (these are credentials):

| Name | What it is |
| --- | --- |
| `AZURE_TENANT_ID` | The directory id of the Azure tenant |
| `AZURE_CLIENT_ID` | The app registration's application id |
| `AZURE_CLIENT_SECRET` | That app registration's client secret |

**Variables** (these are not secret):

| Name | Example |
| --- | --- |
| `AZURE_TRUSTED_SIGNING_ENDPOINT` | `https://weu.codesigning.azure.net` |
| `AZURE_TRUSTED_SIGNING_ACCOUNT` | the Trusted Signing account's name |
| `AZURE_TRUSTED_SIGNING_PROFILE` | the certificate profile's name |

The app registration needs the **Trusted Signing Certificate Profile
Signer** role on the signing account, which is given in the Azure
portal under the account's Access control.

## Checking it worked

Right-click the installer, Properties, Digital Signatures: the signer
is the name on the certificate profile and the timestamp is from
`timestamp.acs.microsoft.com`. SmartScreen stops warning once the
signature has been seen enough times, which is the point of signing
with a certificate that has reputation behind it rather than a fresh
one.

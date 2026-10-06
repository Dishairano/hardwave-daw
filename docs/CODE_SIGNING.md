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

## Doing it, step by step

Everything below happens once. Steps 1 to 5 are in the Azure portal
at https://portal.azure.com, step 6 is in GitHub. Set aside half an
hour, plus the wait in step 3, which is Microsoft checking that
Hardwave Studios is a real company.

### 1. Make sure the subscription can be billed

Trusted Signing is billed per month on an Azure subscription. In the
portal, search **Subscriptions** and open the one you want this on.
It has to say a payment method is attached; a Visual Studio or
Microsoft 365 subscription on its own cannot carry it. Basic is about
USD 10 a month and covers far more signatures than we will use.

Note the subscription's name: step 2 asks for it.

### 2. Create the Trusted Signing account

Search **Trusted Signing accounts** in the portal, then **Create**.

| Field | What to put |
| --- | --- |
| Subscription | the one from step 1 |
| Resource group | create one, `hardwave-signing` |
| Account name | `hardwave` |
| Region | West Europe |
| Pricing tier | Basic |

Create it, wait for the deployment, then **Go to resource**. On the
Overview page there is an **Account URI**, something like
`https://weu.codesigning.azure.net`. Copy it: that is
`AZURE_TRUSTED_SIGNING_ENDPOINT`. The account name, `hardwave`, is
`AZURE_TRUSTED_SIGNING_ACCOUNT`.

### 3. Prove the company is real (the slow bit)

Inside the Trusted Signing account, open **Identity validations** and
create one of type **Organization**. It asks for:

- the legal company name exactly as it is registered,
- the address on the registration,
- the KvK number, as the business registration,
- a company email address and a phone number that can be reached.

Microsoft checks this against public records and sometimes calls or
emails. It is usually a day or two and can be up to a week. Nothing
else can be done until it says **Completed**, so start it first.

What goes in the certificate's subject, and so what Windows shows the
user as the publisher, is the legal name you give here.

### 4. Create the certificate profile

Once the identity validation is Completed: **Certificate profiles** >
**Create**.

| Field | What to put |
| --- | --- |
| Profile type | Public Trust |
| Profile name | `hardwave-daw` |
| Identity validation | the one from step 3 |

`hardwave-daw` is `AZURE_TRUSTED_SIGNING_PROFILE`.

### 5. Give GitHub an account to sign with

GitHub needs its own identity in Azure, so nothing is tied to your
personal login.

**a. Make the app registration.** Search **Microsoft Entra ID** >
**App registrations** > **New registration**. Name it
`hardwave-github-signing`, leave the account type at "this
organizational directory only", and register. On its Overview page
copy:

- **Application (client) ID** → `AZURE_CLIENT_ID`
- **Directory (tenant) ID** → `AZURE_TENANT_ID`

**b. Give it a secret.** On the same app: **Certificates & secrets** >
**New client secret**. Description `github-actions`, expiry 24 months.
Copy the **Value** the moment it appears, not the Secret ID: the value
is shown once and never again. That is `AZURE_CLIENT_SECRET`.

Put a note in your calendar for a month before it expires, because a
secret that runs out means the next release is unsigned and nothing
says why until it happens.

**c. Let it sign.** Back in the Trusted Signing account:
**Access control (IAM)** > **Add** > **Add role assignment**. Role
**Trusted Signing Certificate Profile Signer**, members: the
`hardwave-github-signing` app registration. Review and assign.

Without this one step everything else is in place and every signing
call is refused, so it is worth checking it saved.

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

### 6. Put the six values in GitHub

Open https://github.com/Dishairano/hardwave-daw/settings/secrets/actions

Under **Repository secrets** > **New repository secret**, three times:

- `AZURE_TENANT_ID`
- `AZURE_CLIENT_ID`
- `AZURE_CLIENT_SECRET`

Then the **Variables** tab > **New repository variable**, three times:

- `AZURE_TRUSTED_SIGNING_ENDPOINT`
- `AZURE_TRUSTED_SIGNING_ACCOUNT`
- `AZURE_TRUSTED_SIGNING_PROFILE`

Names are case sensitive and must match exactly. Nothing else has to
be changed in the repository: the next release build picks them up.

## Checking it worked

Download the installer from the next release and right-click it >
Properties > **Digital Signatures**. It should list one signature,
the signer should be the legal name from step 3, and the timestamp
should come from `timestamp.acs.microsoft.com`.

In the build log, the Windows job prints one `sign-windows: signed
...` line per file. If it prints `no Azure Trusted Signing settings`
instead, a variable name is wrong or missing; if it prints `no Azure
credentials`, one of the three secrets is.

SmartScreen softens as the signature is seen on more machines.
Trusted Signing certificates carry Microsoft's own reputation chain,
so this is days rather than the months a fresh certificate of our own
would have taken, but the first few downloads can still show a
warning.

## When something goes wrong

| What the log says | What it means |
| --- | --- |
| `no Azure Trusted Signing settings` | one of the three variables is missing or misspelled |
| `no Azure credentials` | one of the three secrets is missing |
| `AuthorizationFailed` or 403 | step 5c was not done, or was done on the wrong resource |
| `Certificate profile not found` | the profile name does not match `AZURE_TRUSTED_SIGNING_PROFILE` |
| `identity validation` in the message | step 3 has not finished yet |

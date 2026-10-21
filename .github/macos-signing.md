# macOS signing & notarization

The `build-macos` job in [`workflows/release.yml`](workflows/release.yml):

1. Developer-ID-signs `sde` and `libsharpdx.dylib` with the **hardened runtime**.
2. Notarizes those loose binaries (so the `.tar.gz` passes Gatekeeper online — a
   tarball can't carry a stapled ticket).
3. Builds a **Distribution `.pkg`** ([`scripts/macos-pkg.sh`](scripts/macos-pkg.sh))
   that installs `sde` to `/usr/local/bin` (all users) or `~/.local/bin` (current
   user, adding it to `~/.zshrc`), signs it with **Developer ID Installer**, then
   **notarizes + staples** it.

Team ID `7Q3QN9AV9J`. Bundle identifier `ch.erzberger.SharpDataExchange`.

## One-time setup

### 1. Developer ID Installer certificate

The repo already has a *Developer ID Application* cert; a `.pkg` also needs a
*Developer ID Installer* cert.

**Developer portal, not Xcode:** create it with the **G2 Sub-CA**, as listed by
`renew-certs guide` ([ReleaseIt](https://github.com/tinue/ReleaseIt)). Xcode's
*Manage Certificates* still issued certificates from the original Developer ID
Sub-CA in October 2026, and those all stop working on 2027-02-01. Confirm:

```sh
security find-identity -v | grep "Developer ID Installer"
# -> "Developer ID Installer: Martin Erzberger (7Q3QN9AV9J)"
```

**Portal + CSR** (if Xcode doesn't offer it):

1. Keychain Access → *Certificate Assistant* → *Request a Certificate From a
   Certificate Authority…* — your Apple ID email, CN
   `Martin Erzberger Developer ID Installer`, **Saved to disk** + **Let me specify
   key pair information** → RSA 2048 → save the `.certSigningRequest`.
2. <https://developer.apple.com/account/resources/certificates/list> → **+** →
   *Software* → **Developer ID Installer** → upload the CSR → download
   `developerID_installer.cer`.
3. Double-click the `.cer` to import into the **login** keychain (pairs with the
   CSR private key). Re-run the `security find-identity` check above.

### 2. Export both certs as `.p12`

Keychain Access → **login → My Certificates**. For each of *Developer ID
Application: …* and *Developer ID Installer: …* — expand it to confirm a private
key is attached, right-click the **certificate** row → **Export** → *Personal
Information Exchange (.p12)*. Use the **same** export password for both:

- `DeveloperIDApplication.p12`
- `DeveloperIDInstaller.p12`

`renew-certs apply` does this export itself and sets the two `_P12_BASE64`
secrets and `APPLE_CERT_P12_PASSWORD` (see *Rotating*).

### 3. App Store Connect API key (for notarization)

<https://appstoreconnect.apple.com> → *Users and Access* → *Integrations* →
*App Store Connect API* → *Team Keys* → **Generate API Key**, role **Developer**.
Download `AuthKey_XXXXXXXXXX.p8` (**one download only** — back it up). Record the
**Key ID** and the **Issuer ID** (UUID at the top of the Keys list).

> If notarization later fails with a licensing error, sign in to the developer
> portal once and accept the pending Program License Agreement. A brand-new key
> can also take a few minutes before `notarytool` accepts it.

### 4. Repo secrets

`Settings → Secrets and variables → Actions`, or `gh` (run where the three
downloaded files are — macOS `base64` does not line-wrap):

```sh
gh secret set APPLE_CERT_APPLICATION_P12_BASE64 < <(base64 -i DeveloperIDApplication.p12)
gh secret set APPLE_CERT_INSTALLER_P12_BASE64   < <(base64 -i DeveloperIDInstaller.p12)
gh secret set APPLE_API_KEY_P8_BASE64           < <(base64 -i AuthKey_XXXXXXXXXX.p8)
printf %s 'THE_EXPORT_PASSWORD'                  | gh secret set APPLE_CERT_P12_PASSWORD
printf %s 'ABCDE12345'                           | gh secret set APPLE_API_KEY_ID
printf %s '69a6de7f-1a2b-3c4d-5e6f-70a1b2c3d4e5' | gh secret set APPLE_API_ISSUER_ID
```

| Secret | Contents |
|---|---|
| `APPLE_CERT_APPLICATION_P12_BASE64` | base64 of `DeveloperIDApplication.p12` |
| `APPLE_CERT_INSTALLER_P12_BASE64`   | base64 of `DeveloperIDInstaller.p12` |
| `APPLE_CERT_P12_PASSWORD`           | the `.p12` export password (same for both) |
| `APPLE_API_KEY_P8_BASE64`           | base64 of `AuthKey_XXXXXXXXXX.p8` |
| `APPLE_API_KEY_ID`                  | the key's ID, e.g. `ABCDE12345` |
| `APPLE_API_ISSUER_ID`               | the issuer UUID |

Then delete the local `.p12` files (the certs can be re-exported from Keychain;
keep the `.p8` backed up — it can't be re-downloaded).

## Rotating

Developer ID certs from the G2 Sub-CA are valid only ~13 months. Each year:
create new ones in the developer portal (`renew-certs guide`), then run
`renew-certs apply` in the repo root (team ID from the `certs:` section of
`release.yaml`). It exports them from the keychain, checks them, and sets
`APPLE_CERT_APPLICATION_P12_BASE64`, `APPLE_CERT_INSTALLER_P12_BASE64` and
`APPLE_CERT_P12_PASSWORD`. Run it in Calc-U-1600 too; it uses the same pair.
It also records the uploaded certificates as the repo
variables `APPLE_CERT_{APPLICATION,INSTALLER}_{SHA1,EXPIRES}`;
`renew-certs status` reports them and their expiry. The `.p12` password is random per
run and kept nowhere else; if a secret breaks, run `apply` again. Back the
certificates themselves up with `renew-certs backup` (1Password, Private
vault; restore by double-clicking the attached `.p12`).

The App Store Connect API key doesn't expire. For a new one, replace the `.p8`
secret plus `APPLE_API_KEY_ID` / `APPLE_API_ISSUER_ID`.

## Local dry run (no secrets)

```sh
cargo build --release --target aarch64-apple-darwin
.github/scripts/macos-pkg.sh target/aarch64-apple-darwin/release/sde 0.0.0 /tmp/sde-test.pkg
pkgutil --expand-full /tmp/sde-test.pkg /tmp/sde-test-expand   # inspect payload + scripts
```

With `INSTALLER_IDENTITY` unset the script emits an **unsigned** `.pkg` — fine for
inspecting `pkgbuild`/`productbuild` output, not installable on other Macs.

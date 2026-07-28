# Release Process

This document is for maintainers publishing prebuilt `stock-tui` binaries.
Local source builds do not require signing credentials.

## Release Outputs

The release workflow builds five target archives. Every run also produces one
signed, notarized, and stapled disk image for each macOS architecture:

| Target | Archive | Signed disk image |
| --- | --- | --- |
| Apple Silicon | `stock-tui-v<VERSION>-aarch64-apple-darwin.tar.gz` | `stock-tui-v<VERSION>-aarch64-apple-darwin.dmg` |
| Intel | `stock-tui-v<VERSION>-x86_64-apple-darwin.tar.gz` | `stock-tui-v<VERSION>-x86_64-apple-darwin.dmg` |

The macOS tarballs and disk images contain the same Developer ID-signed
command-line binary. The disk images carry stapled notarization tickets and
are the preferred macOS downloads. Tag runs publish all artifacts and a
`SHA256SUMS` file to GitHub Releases. Manually dispatched runs perform the same
signing and Apple validation but retain the results as short-lived workflow
artifacts instead of publishing a release.

Apple requires directly distributed macOS software to use a Developer ID
certificate, a secure timestamp, and the hardened runtime before
notarization. See Apple's
[notarization requirements](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)
and
[custom notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).

## Apple Credentials

Create a `Developer ID Application` certificate and export the identity,
including its private key, from Xcode or Keychain Access as a
password-protected PKCS#12 file. A `Developer ID Installer` certificate is not
needed because releases use disk images rather than installer packages.
Apple documents certificate creation in
[Developer ID certificates](https://developer.apple.com/help/account/certificates/create-developer-id-certificates)
and PKCS#12 export in
[Synchronizing code signing identities](https://developer.apple.com/documentation/Xcode/sharing-your-teams-signing-certificates).

Create a team App Store Connect API key for `notarytool`. Do not use an
individual API key; Apple states that individual keys cannot access
`notarytool`. Record its key ID and issuer ID when the key is created, because
the private `.p8` key can be downloaded only once. See
[Creating API keys for App Store Connect API](https://developer.apple.com/documentation/appstoreconnectapi/creating-api-keys-for-app-store-connect-api).

Create a GitHub Actions environment named `macos-release`, optionally require
a trusted reviewer, and configure these environment secrets:

| Secret | Value |
| --- | --- |
| `MACOS_SIGNING_CERT_P12_BASE64` | Single-line Base64 encoding of the exported `.p12` |
| `MACOS_SIGNING_CERT_PASSWORD` | Password protecting the `.p12` |
| `APPLE_NOTARY_API_KEY_P8_BASE64` | Single-line Base64 encoding of the team API `.p8` |
| `APPLE_NOTARY_KEY_ID` | App Store Connect team API key ID |
| `APPLE_NOTARY_ISSUER_ID` | App Store Connect API issuer UUID |

Base64 is transport encoding, not encryption. Keep the original files and
password in an appropriate secret manager, never commit them, and revoke a
certificate or API key immediately if exposure is suspected. Restrict
repository and Actions administration to trusted maintainers. Protect the
`v*` tag namespace with a GitHub ruleset so only release maintainers can start
a credential-bearing tagged build. Environment protection rules must permit
the release tags and branches from which maintainers run the workflow.

The binary key material is encoded and streamed directly into GitHub CLI with:

```bash
base64 < DeveloperIDApplication.p12 \
  | tr -d '\n' \
  | gh secret set MACOS_SIGNING_CERT_P12_BASE64 \
      --env macos-release \
      --repo chatcode-lab/stock-tui

base64 < AuthKey_NOTARY.p8 \
  | tr -d '\n' \
  | gh secret set APPLE_NOTARY_API_KEY_P8_BASE64 \
      --env macos-release \
      --repo chatcode-lab/stock-tui
```

Set the other three values through GitHub's encrypted secret prompt or the
`macos-release` environment settings. The secret names match the private
Chatcode release workflow, but GitHub does not allow one repository to read or
copy another repository's secrets. Upload the original credentials directly
to this environment. Do not place any credential in workflow inputs,
repository variables, command history, release assets, or issue comments.

```bash
gh secret set MACOS_SIGNING_CERT_PASSWORD \
  --env macos-release --repo chatcode-lab/stock-tui
gh secret set APPLE_NOTARY_KEY_ID \
  --env macos-release --repo chatcode-lab/stock-tui
gh secret set APPLE_NOTARY_ISSUER_ID \
  --env macos-release --repo chatcode-lab/stock-tui
```

## Tagged Release

1. Update `Cargo.toml` and `CHANGELOG.md`, then run all checks in
   [CONTRIBUTING.md](../CONTRIBUTING.md).
2. Push the reviewed release commit.
3. Create and push the exact `v<VERSION>` tag.
4. Watch the `Release` workflow. A missing or invalid Apple credential fails
   both macOS release jobs before publication.
5. Confirm that both macOS jobs report accepted notarization and successful
   stapling before treating the GitHub release as complete.

The workflow imports the signing identity into an ephemeral keychain, signs
the executable with a stable identifier, secure timestamp, and hardened
runtime, and rejects a true `com.apple.security.get-task-allow` entitlement.
It creates and signs the disk image, verifies its UDIF structure, submits it
with `notarytool`, then downloads the JSON submission log. It requires
`Accepted` in both responses and rejects any error-level issue before stapling
and validating the ticket. Temporary key material, response logs, and the
keychain are removed when the step exits.

## Independent Verification

Download a macOS disk image and `SHA256SUMS`, then verify them on macOS:

```bash
artifact=stock-tui-v<VERSION>-aarch64-apple-darwin.dmg
grep "  ${artifact}$" SHA256SUMS | shasum -a 256 --check
xcrun stapler validate "$artifact"
spctl --assess --type open --context context:primary-signature \
  --verbose=2 "$artifact"
```

Mount the image and inspect the command-line executable:

```bash
codesign --verify --strict --verbose=2 /Volumes/stock-tui*/stock-tui
codesign --display --verbose=4 /Volumes/stock-tui*/stock-tui
codesign --display --entitlements :- /Volumes/stock-tui*/stock-tui
```

The signature details should identify a Developer ID Application authority,
show the `runtime` flag, include a timestamp, and omit a true
`com.apple.security.get-task-allow` entitlement. Do not publish a macOS release
when any of these checks fail.

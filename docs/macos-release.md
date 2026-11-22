# Signed macOS release candidate

The **Signed macOS candidate** workflow builds the Apple Silicon installer on `main`, signs the app with Developer ID, and notarizes and staples the disk image. It uploads a candidate artifact after Gatekeeper accepts both the image and the enclosed app.

## Set up signing

Configure these repository Actions secrets in GitHub, under **Settings → Secrets and variables → Actions**. Keep values in GitHub or your own secure storage.

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of a Developer ID Application `.p12` export containing the certificate and its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password used to export the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full Keychain identity, beginning with `Developer ID Application:` |
| `APPLE_API_ISSUER` | Team API issuer ID in App Store Connect |
| `APPLE_API_KEY` | ID of the App Store Connect team API key for notarization |
| `APPLE_API_KEY_P8` | Contents of the corresponding `.p8` key |

Run **Signed macOS candidate** from Actions on `main`. Download the resulting DMG. On a clean Apple Silicon Mac account, open the image, drag Keel Desktop to Applications, and launch from Finder. Use guided setup to get a real reply, quit normally, then reopen and get a reply offline. Record the release commit, hardware, macOS version, model, and result. Repeat with an interrupted download and confirm that it resumes.

The candidate is a workflow artifact. Use its exact DMG for publication after verifying the clean-account flow. Update the download page once the signed artifact is live.

[Apple notarization documentation](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution) · [Tauri signing configuration](https://v2.tauri.app/distribute/sign/macos/)

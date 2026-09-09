# Changelog

## 0.1.1

- Publish under `onenote-cli` on PyPI, matching the crates.io package name.
- Update wheel verification, installation instructions, and migration guidance.
- Preserve the existing `onenote-cli-rs` 0.1.0 package and all 0.1.0 artifacts.
- The executable is still named `onenote`; desktop behavior is unchanged.

## 0.1.0

Initial Windows desktop release of `onenote-cli`.

- Discover open notebooks, nested sections, and pages with IDs and hierarchy paths.
- Read page text as Markdown, with optional original XML.
- Search titles and page text without Windows indexing using `search --scan`.
  Scans use literal matching, bounded batches, and explicit coverage metadata.
- Keep OneNote's indexed search available for its native search syntax.
- Create plain-text pages and append separate text blocks with timestamp checks,
  read-back verification, and explicit uncertain-write errors.
- Provide terminal and JSON output, field selection, pagination, shell
  completions, diagnostics, and an offline CLI Spec contract.

Requires Windows OneNote desktop and Windows PowerShell 5.1 in the existing
OneNote user's session. No Graph app registration or separate OAuth consent.
The executable archive targets x86-64 Windows. Binary wheels for Windows x64
and Linux x64 allow uv installation without Rust. The Linux desktop backend
is intended for WSL interop; live WSL2 validation is still outstanding. There is no TUI in this release.

Known limitations: indexed search remains broken on the test kiosk despite
Office repair and a reboot; use `--scan`. Scans search extracted text, not ink,
OCR, or attachment contents, and report locked or unreadable content as skipped.
Reads and verified writes reflect desktop state, not guaranteed cloud sync.

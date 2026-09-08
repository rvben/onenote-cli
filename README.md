# onenote-cli

Use your OneNote notebooks from the terminal through the **Windows desktop
application**, using its existing signed-in session. No Graph connection, Entra
app registration, or separate OAuth consent is required by this backend.

Discover notebooks and sections, list and search pages, read page content, and
create or append plain-text notes. It has readable terminal output, JSON when
piped, bounded collections, shell completions, and an offline CLI Spec contract.

## Requirements

- Windows OneNote desktop with your notebooks already open and accessible.
- Windows PowerShell 5.1 (`powershell.exe`) on PATH.
- Run under the Windows user who uses OneNote. WSL is supported through Windows
  interop; OneNote still runs on the Windows side.

OneNote for macOS and the browser do not expose this COM interface. Build and
offline commands work on other platforms. Existing notebook permissions,
password protection, and organizational desktop policies still apply.

## Build and start

Requires Rust 1.90 or later. From this directory:

```console
cargo install --path . --locked
onenote doctor
onenote notebooks list
onenote sections list --notebook '{NOTEBOOK-ID}'
onenote pages list --section '{SECTION-ID}'
onenote pages read '{PAGE-ID}'
onenote search 'quarterly review' --scan
```

Copy IDs from the preceding command. These are desktop OneNote IDs, not Graph
IDs. Quote them in your shell. Sections inside section groups are included and
identified by their hierarchy path. Recycle-bin content is excluded from lists.

```console
onenote search 'budget AND review' --scope '{NOTEBOOK-ID}' --limit 10
onenote pages list --section '{SECTION-ID}' --limit 10 --offset 10
onenote pages list --section '{SECTION-ID}' --fields id,name --output json
onenote pages read '{PAGE-ID}' --xml --output json
onenote doctor --offline
onenote schema --command 'pages read'
onenote completions powershell
```

Collections return `items`, `total`, `next_offset`, and `truncated`. Reuse the same
command and filters with `--offset` set to `next_offset`; stop when it is null.
The default limit is 25, maximum 100. Pagination is positional: changes between
requests can shift items. OneNote supplies the hierarchy before local pagination;
the bridge response is capped at 32 MiB and commands time out after 45 seconds.

Page results include `markdown`, extracted from OneNote text blocks. This is not
a layout-preserving export: tables, tags, and outline nesting may lose structure;
ink, images, and attachment bytes are not exported. `--xml` includes the original
page XML without binary payloads. Reads reflect desktop state and do not force
cloud synchronization. Search uses OneNote's search syntax and requires the
Windows Search (`WSearch`) service to be running. It does not navigate the desktop
UI. Unindexed pages are excluded from matches: results include `indexing_pending`
and `unindexed_count` so an empty result cannot hide pending indexing. New or
changed pages may take time to become searchable. Listing and reading pages do
not depend on the search index.

### Search without Windows indexing

```console
onenote search 'quarterly review' --scan --scope '{SECTION-ID}'
onenote search 'quarterly review' --scan --scope '{NOTEBOOK-ID}' --scan-limit 100
onenote search 'quarterly review' --scan --scope '{NOTEBOOK-ID}' --scan-offset 100
```

`--scan` reads titles and page text directly, with case-insensitive literal
substring matching and whitespace normalization. It does not interpret Boolean
operators, wildcards, or OneNote search syntax. HTML formatting is removed and
entities decoded; link destinations, ink, OCR, and attachment content are not
searched. It needs no Windows Search service. Use a section scope for fast,
focused searches; without a scope it scans all open notebooks.

Each request attempts at most 100 pages by default (`--scan-limit` accepts 1–500).
A scan stops starting page reads after 20 seconds or when its XML response budget
reaches 4,194,304 UTF-16 code units (before JSON escaping). An individual blocked COM call can still hit
the bridge's 45-second timeout, which returns an error rather than partial results.
Oversized pages and read failures appear in `skipped_pages`; locked sections are
never read and are counted in `skipped_locked_sections`.

`total` counts matches in the current scan batch. `--limit`, `--offset`, and
`next_offset` paginate those matches. After consuming them, use
`next_scan_offset` as `--scan-offset` and reset `--offset` to zero to scan the next
batch. Keep the query, scope, and scan limit unchanged. These cursors are
positional, not snapshots; edits or timing differences between requests can shift
results. Narrow the scope if a scan cannot advance its cursor.

`incomplete: true` means there are more candidate pages, skipped pages, or locked
sections. `next_scan_offset: null` means no more candidates remain, but skipped
content can still make results incomplete. `scanned_pages`, `attempted_pages`,
and `candidate_pages` describe coverage; candidate counts exclude locked and
recycled pages. Partial results exit successfully with this metadata. Terminal
output also explicitly labels incomplete results.

`--output auto` selects text on a terminal and JSON when piped. Use `-o text` to
force readable text or `-o json` for automation. `--no-color` and `NO_COLOR`
disable color. Errors go to stderr as structured JSON in JSON mode, leaving
stdout empty. Exit codes: 1 local failure, 2 invalid input/unsupported platform,
4 resource not found, 5 desktop bridge failure, 6 write conflict, 7 uncertain write. OneNote COM failures, including
inaccessible IDs and locked sections, are reported as desktop errors with the
original diagnostic. Locked-section errors also explain how to unlock the section
in the desktop application.

## Create and append

```console
onenote pages create --section '{SECTION-ID}' --title 'Meeting notes' --file notes.txt --dry-run
onenote pages create --section '{SECTION-ID}' --title 'Meeting notes' --file notes.txt
onenote pages append '{PAGE-ID}' --text 'Follow up on the proposal.'
cat notes.txt | onenote pages append '{PAGE-ID}' --file -
```

Supply exactly one of `--text` or `--file`. Content is UTF-8 plain text, limited
to 1 MiB; titles are limited to 512 bytes. Markdown and HTML are treated as
literal text. Input line breaks become separate OneNote text elements. A UTF-8
BOM is removed and CRLF/CR line endings are normalized. `--dry-run` works offline
and prints the request without checking whether the target exists or is writable.

Create and append are explicit write commands and do not ask for confirmation.
Append places a new outline below existing content, without sending existing
outlines, images, or attachments back to OneNote. Every update uses the page's
last-modified timestamp and `force=false`. A concurrent edit returns `conflict`
(exit 6) rather than overwriting content.

Success includes the page ID, the resulting Markdown, `operation`, and
`verified: true`. Verification reads the page back, checks the new text (ignoring
rendering whitespace differences), and checks that existing page-level object IDs
remain. Create also checks the title. This verifies desktop state, not cloud sync.

These operations are **not idempotent**: repeating a successful command creates
another page or appends another block. Creation is a two-step desktop operation;
a failure after creating the page can leave a blank or partially populated page.
A timeout, interrupted response, or failed verification returns `write_uncertain`
(exit 7). Inspect OneNote before retrying. When known, the error includes
`page_id` for recovery. The CLI never retries a write or automatically deletes a
partially created page.

## Implementation and validation

The Rust CLI passes JSON over stdin to a bundled PowerShell script. User text is
never interpolated into executable code. Read commands use `GetHierarchy`,
`FindPages`, and `GetPageContent`. The separate write bridge adds `CreateNewPage`
and `UpdatePageContent`. Both release their COM reference and never quit the
user's OneNote application.

```console
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

On Windows, tests exercise the production script using a mock COM object. On
other platforms with PowerShell installed:

```console
ONENOTE_TEST_POWERSHELL=/path/to/pwsh cargo test --test desktop_bridge --test desktop_write -- --ignored
```

Mock tests cannot verify a real OneNote installation. Before release, run
`doctor`, notebook/section/page discovery, search, and page reading against a
Windows desktop profile, including a nested section and a locked section.

The initial kiosk smoke test passed discovery, nested sections, pagination,
field projection, Unicode page reads, Markdown/XML output, and pending-index
search behavior. Live create/append checks also passed for Unicode, literal
special characters, line breaks, existing-content preservation, and dry-run.
A real password-locked section was verified: discovery reports `locked: true`,
page listing requests an unlock, and creation is refused before mutation.
Page read/append errors for locked content are covered by COM-error fixtures.
Live `--scan` checks passed positive title and body matches, Unicode, negative
queries, bounded continuation, and locked-section omission with incomplete-result
metadata.
Indexed search remains an environment limitation: the kiosk's OneNote
crawler currently fails with `0x80004005` despite a running Windows Search service
and a targeted re-crawl. A normal OneNote restart and Office Quick Repair also
completed without resolving the crawler failure. A full kiosk reboot cleared
pending Windows servicing but did not fix indexing. Post-reboot checks passed
doctor, scratch-page discovery and reading, and confirmed that LockedSmoke
remains locked. Windows Search now uses delayed automatic startup on the test
kiosk. It must remain
enabled in the kiosk configuration; a provisioning run that disables `WSearch`
will break search again.

Microsoft references: [Application interface](https://learn.microsoft.com/en-us/office/client-developer/onenote/application-interface-onenote)
and [COM enumerations](https://learn.microsoft.com/en-us/office/client-developer/onenote/enumerations-onenote-developer-reference).

## Prepare a release locally

The initial standalone package targets x86-64 Windows. On Windows, build and
package it with:

```console
cargo build --release --locked --target x86_64-pc-windows-msvc
python scripts/package.py --target x86_64-pc-windows-msvc --output dist
```

Cross-builds with `cargo zigbuild --release --locked --target
x86_64-pc-windows-gnu` can be packaged with the corresponding GNU target.
The script verifies the executable architecture and archive contents, includes
this README, the license, and changelog, and writes a SHA-256 sidecar. Extract the
archive and verify `onenote.exe --version`, `doctor`, and the live read/write
smoke checks on Windows before publishing. Packaging does not publish or tag.

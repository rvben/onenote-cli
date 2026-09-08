# onenote-cli

Read your OneNote notebooks from the terminal through the **Windows desktop
application**, using its existing signed-in session. No Graph connection, Entra
app registration, or separate OAuth consent is required by this backend.

This first version is read-only: notebook and section discovery, page lists,
full-text search, and page reading. It has readable terminal output, JSON when
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
onenote search 'quarterly review'
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

`--output auto` selects text on a terminal and JSON when piped. Use `-o text` to
force readable text or `-o json` for automation. `--no-color` and `NO_COLOR`
disable color. Errors go to stderr as structured JSON in JSON mode, leaving
stdout empty. Exit codes: 1 local failure, 2 invalid input/unsupported platform,
4 resource not found, 5 desktop bridge failure. OneNote COM failures, including
inaccessible IDs and locked sections, are reported as desktop errors with the
original diagnostic. Locked-section errors also explain how to unlock the section
in the desktop application.

## Implementation and validation

The Rust CLI passes JSON over stdin to a bundled PowerShell script. User text is
never interpolated into executable code. The script calls only `GetHierarchy`,
`FindPages`, and `GetPageContent`, releases its COM reference, and never quits
the user's OneNote application.

```console
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

On Windows, tests exercise the production script using a mock COM object. On
other platforms with PowerShell installed:

```console
ONENOTE_TEST_POWERSHELL=/path/to/pwsh cargo test --test desktop_bridge -- --ignored
```

Mock tests cannot verify a real OneNote installation. Before release, run
`doctor`, notebook/section/page discovery, search, and page reading against a
Windows desktop profile, including a nested section and a locked section.

The initial kiosk smoke test passed discovery, nested sections, pagination,
field projection, Unicode page reads, Markdown/XML output, and pending-index
search behavior. Positive indexed search and a real password-locked section
remain unverified; the latter is covered by XML and COM-error fixtures. Treat
those two live checks as release prerequisites. Windows Search must remain
enabled in the kiosk configuration; a provisioning run that disables `WSearch`
will break search again.

Microsoft references: [Application interface](https://learn.microsoft.com/en-us/office/client-developer/onenote/application-interface-onenote)
and [COM enumerations](https://learn.microsoft.com/en-us/office/client-developer/onenote/enumerations-onenote-developer-reference).

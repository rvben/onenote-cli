use crate::{error::CONTRACTS, model::FIELDS};
use serde_json::{Value, json};

fn arg(name: &str, kind: &str, required: bool) -> Value {
    json!({"name":name,"type":kind,"required":required})
}
pub fn capabilities() -> Value {
    json!({"backend":"desktop", "authentication":"existing Windows OneNote desktop session", "backends":["desktop","ssh"], "platforms":["windows","wsl","macos","linux"],
        "requires":"Windows OneNote desktop and Windows PowerShell 5.1 on the target. Local WSL requires Windows interop; remote access requires OpenSSH and permission to register an interactive task as the signed-in Windows user", "writes":true,
        "supported":["notebooks list","sections list","pages list","pages read","search","pages create","pages append"],
        "limitations":["SSH uses key/agent authentication and existing trusted host keys", "Read-only profiles refuse writes; dry-run remains available", "Only notebooks open in the desktop application", "Reads desktop state; does not guarantee fresh cloud sync", "Page Markdown is a text extraction; layout, ink, and binary content are not exported", "Pagination is positional; edits between requests can shift results", "COM retrieves a hierarchy before local pagination; bridge output is capped at 32 MiB", "Indexed search requires Windows Search; unindexed pages are omitted and reported via indexing_pending and unindexed_count", "search --scan reads titles and text directly with case-insensitive literal, whitespace-normalized matching; no ink, OCR, or attachment search", "Scans default to 100 pages, maximum 500, with a 20-second soft budget and 45-second bridge timeout; use next_scan_offset separately from result pagination; incomplete reports unread pages or locked sections", "Writes accept plain text up to 1 MiB; no Markdown or HTML interpretation", "Create and append are not idempotent; inspect OneNote before retrying any uncertain result", "Write verification checks desktop state, not cloud synchronization"]})
}
pub fn generate(filter: Option<&str>) -> Value {
    let mut commands = Vec::new();
    for (name, description, extra) in [
        ("notebooks list", "List open notebooks", vec![]),
        (
            "sections list",
            "List sections recursively within a notebook",
            vec![arg("--notebook", "string", true)],
        ),
        (
            "pages list",
            "List pages within a section",
            vec![arg("--section", "string", true)],
        ),
        (
            "search",
            "Search using OneNote syntax, or literal text with --scan",
            vec![
                arg("query", "string", true),
                arg("--scope", "string", false),
                arg("--scan", "boolean", false),
                arg("--scan-limit", "integer", false),
                arg("--scan-offset", "integer", false),
            ],
        ),
    ] {
        let mut args = vec![
            arg("--limit", "integer", false),
            arg("--offset", "integer", false),
            arg("--fields", "string", false),
        ];
        args.extend(extra);
        commands.push(json!({"name":name,"description":description,"effects":"read_only","cardinality":"unbounded",
            "pagination":{"style":"offset","offset_arg":"--offset","limit_arg":"--limit"},"fields_arg":"--fields","args":args,
            "output_fields":FIELDS.iter().map(|f| json!({"name":f,"type":if *f == "locked" {"boolean"} else {"string"},"nullable":matches!(*f,"parent_id"|"last_modified")})).collect::<Vec<_>>()}));
    }
    for (name, description, mut args) in [
        (
            "pages create",
            "Create a page with verified plain-text content",
            vec![
                arg("--section", "string", true),
                arg("--title", "string", true),
            ],
        ),
        (
            "pages append",
            "Append a new text block with a modification-time check and read-back verification",
            vec![arg("id", "string", true)],
        ),
    ] {
        args.extend([
            arg("--text", "string", false),
            arg("--file", "string", false),
            arg("--dry-run", "boolean", false),
        ]);
        commands.push(json!({"name":name,"description":description,"effects":"non_idempotent","cardinality":"single","args":args,"stdout_schema":{"type":"object"}}));
    }
    for (name, description, args) in [
        (
            "pages read",
            "Read page text as Markdown; optionally include XML",
            vec![arg("id", "string", true), arg("--xml", "boolean", false)],
        ),
        (
            "doctor",
            "Diagnose the local platform and desktop bridge",
            vec![arg("--offline", "boolean", false)],
        ),
        (
            "capabilities",
            "Describe desktop capabilities and limitations",
            vec![],
        ),
        (
            "schema",
            "Emit the offline CLI Spec v0.3 contract",
            vec![arg("--command", "string", false)],
        ),
    ] {
        commands.push(json!({"name":name,"description":description,"effects":"read_only","cardinality":"single","stdout_schema":{"type":"object"},"args":args}));
    }
    commands.push(json!({"name":"completions","description":"Generate shell completions","effects":"read_only","output_kind":"opaque","media_type":"text/plain","args":[arg("shell","string",true)]}));
    for (name, description, effects, args) in [
        (
            "init",
            "Check and save a desktop or SSH profile",
            "idempotent",
            vec![
                arg("--backend", "string", false),
                arg("--host", "string", false),
                arg("--identity-file", "string", false),
                arg("--port", "integer", false),
                arg("--read-only", "boolean", false),
                arg("--no-check", "boolean", false),
                arg("--force", "boolean", false),
            ],
        ),
        (
            "profile list",
            "List saved profiles and the active selection",
            "read_only",
            vec![],
        ),
        (
            "profile use",
            "Select the default profile",
            "idempotent",
            vec![arg("name", "string", true)],
        ),
        (
            "profile remove",
            "Remove connection settings",
            "idempotent",
            vec![arg("name", "string", true)],
        ),
        (
            "config show",
            "Show resolved settings without connecting",
            "read_only",
            vec![],
        ),
        (
            "config path",
            "Show the configuration file location",
            "read_only",
            vec![],
        ),
    ] {
        commands.push(json!({"name":name,"description":description,"effects":effects,"cardinality":"single","stdout_schema":{"type":"object"},"args":args}));
    }
    if let Some(filter) = filter {
        commands.retain(|c| c["name"] == filter);
    }
    json!({"$schema":"https://clispec.dev/schema/v0.3.json","clispec":"0.3","name":"onenote","version":env!("CARGO_PKG_VERSION"),
        "description":"Read and capture OneNote notes through the Windows desktop application","output":{"tty":"text","piped":"json"},
        "global_args":[{"name":"--profile","type":"string","description":"Profile override; also ONENOTE_PROFILE"},{"name":"--output","short":"-o","type":"string","enum":["auto","text","json"],"default":"auto"},{"name":"--no-color","type":"boolean"}],
        "commands":commands,"errors":CONTRACTS.iter().map(|(kind,code,description)|json!({"kind":kind,"exit_code":code,"retryable":false,"description":description})).collect::<Vec<_>>(),
        "extensions":capabilities()})
}

use assert_cmd::Command;
use clap::{CommandFactory, Parser};
use onenote_cli::{
    cli::{Cli, Format, PageArgs},
    model, output, schema,
};
use serde_json::Value;

const HIERARCHY: &str = include_str!("fixtures/hierarchy.xml");
fn args(limit: u16, offset: u32) -> PageArgs {
    PageArgs {
        limit,
        offset,
        fields: vec![],
    }
}

#[test]
fn nested_sections_pagination_projection_and_empty_results() {
    let page = model::collection(HIERARCHY, "Section", &args(1, 0)).unwrap();
    assert_eq!(page["total"], 2);
    assert_eq!(page["next_offset"], 1);
    assert_eq!(page["items"][0]["path"], "Work / Meetings");
    let next = model::collection(HIERARCHY, "Section", &args(1, 1)).unwrap();
    assert_eq!(next["items"][0]["path"], "Work / Projects / Planning");
    assert_eq!(next["items"][0]["locked"], true);
    assert_eq!(next["items"][0]["parent_id"], "{group}");
    assert!(next["next_offset"].is_null());
    let empty = model::collection(HIERARCHY, "Page", &args(10, 20)).unwrap();
    assert_eq!(empty["items"], serde_json::json!([]));
    assert_eq!(empty["truncated"], false);
    let projected = model::collection(
        HIERARCHY,
        "Page",
        &PageArgs {
            fields: vec!["id".into()],
            ..args(10, 0)
        },
    )
    .unwrap();
    assert_eq!(projected["items"][0].as_object().unwrap().len(), 1);
    assert!(
        model::collection(
            HIERARCHY,
            "Page",
            &PageArgs {
                fields: vec!["typo".into()],
                ..args(10, 0)
            }
        )
        .is_err()
    );
}
#[test]
fn search_never_treats_unindexed_pages_as_matches() {
    let xml = r#"<one:Section xmlns:one="http://schemas.microsoft.com/office/onenote/2013/onenote" ID="section" name="Test">
        <one:Page ID="pending" name="Not a match" isIndexed="false"/>
        <one:Page ID="match" name="Match" isIndexed="true"/>
        <one:Page ID="match2" name="Another match"/>
    </one:Section>"#;
    let first = model::search(xml, &args(1, 0)).unwrap();
    assert_eq!(first["items"][0]["id"], "match");
    assert_eq!(first["total"], 2);
    assert_eq!(first["next_offset"], 1);
    assert_eq!(first["unindexed_count"], 1);
    assert_eq!(first["indexing_pending"], true);
    assert!(output::render(&first, false).contains("unindexed pages omitted"));
    assert_eq!(
        model::search(xml, &args(1, 1)).unwrap()["items"][0]["id"],
        "match2"
    );
    assert_eq!(
        model::collection(xml, "Page", &args(10, 0)).unwrap()["total"],
        3
    );
    let only_pending = xml
        .replace("isIndexed=\"true\"", "isIndexed=\"false\"")
        .replace(
            "name=\"Another match\"",
            "name=\"Another match\" isIndexed=\"false\"",
        );
    let empty = model::search(&only_pending, &args(10, 0)).unwrap();
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["unindexed_count"], 3);
    assert_eq!(empty["truncated"], false);
    assert!(empty["next_offset"].is_null());
}

#[test]
fn page_preserves_unicode_links_and_exposes_nontext_content() {
    let xml = include_str!("fixtures/page.xml");
    let page = model::read(xml, "{page1}", true).unwrap();
    let markdown = page["markdown"].as_str().unwrap();
    assert!(markdown.contains("**team**"));
    assert!(markdown.contains("& 日本語"));
    assert!(markdown.contains("[Report](https://example.com/report)"));
    assert!(!markdown.contains("Quarterly review"));
    assert_eq!(page["has_images"], true);
    assert_eq!(page["has_attachments"], true);
    assert_eq!(page["xml"], xml);
    assert!(model::read(xml, "other", false).is_err());
    assert!(
        model::read(xml, "{page1}", false)
            .unwrap()
            .get("xml")
            .is_none()
    );
}
#[test]
fn rejects_malformed_xml_and_dtds() {
    for xml in [
        "garbage",
        "<Page/>",
        "<!DOCTYPE x [<!ENTITY leak SYSTEM 'file:///etc/passwd'>]><x>&leak;</x>",
    ] {
        assert!(model::collection(xml, "Page", &args(10, 0)).is_err());
    }
}
#[test]
fn contract_is_valid_and_matches_all_invocable_commands() {
    Cli::command().debug_assert();
    let spec: Value = serde_json::from_str(include_str!("fixtures/clispec-v0.3.json")).unwrap();
    let contract = schema::generate(None);
    jsonschema::validator_for(&spec)
        .unwrap()
        .validate(&contract)
        .unwrap();
    fn paths(command: clap::Command, prefix: String) -> Vec<String> {
        command
            .get_subcommands()
            .flat_map(|child| {
                let name = format!("{prefix}{}", child.get_name());
                if child.has_subcommands() {
                    paths(child.clone(), format!("{name} "))
                } else {
                    vec![name]
                }
            })
            .collect()
    }
    let mut actual = paths(Cli::command(), String::new());
    let mut declared: Vec<_> = contract["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_owned())
        .collect();
    actual.sort();
    declared.sort();
    assert_eq!(actual, declared);
}
#[test]
fn terminal_output_keeps_ids_and_continuation_and_sanitizes_controls() {
    let page = model::collection(HIERARCHY, "Page", &args(1, 0)).unwrap();
    let text = output::render(&page, false);
    assert!(text.contains("Quarterly review"));
    assert!(text.contains("{page1}"));
    assert!(text.contains("--offset 1"));
    assert!(!text.contains('\x1b'));
    assert_eq!(
        output::clean("hello\x1b\u{009b}\u{202e}\rworld\n"),
        "helloworld\n"
    );
}
#[test]
fn parse_errors_respect_each_output_spelling_and_leave_stdout_empty() {
    for flag in [
        vec!["-o", "json"],
        vec!["-ojson"],
        vec!["-o=json"],
        vec!["--output=json"],
    ] {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("onenote"));
        let result = cmd
            .args(flag)
            .args(["pages", "list", "--limit", "0"])
            .assert()
            .code(2);
        let output = result.get_output();
        assert!(output.stdout.is_empty());
        let error: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["kind"], "invalid_input");
    }
    let args = ["onenote", "-otext", "--", "-ojson"].map(str::to_owned);
    assert_eq!(output::format_from_args(&args), Format::Text);
    assert!(Cli::try_parse_from(["onenote", "search", "   "]).is_err());
    assert!(Cli::try_parse_from(["onenote", "pages", "read", ""]).is_err());
}
#[test]
fn offline_commands_work_without_desktop_and_bad_fields_fail_before_launch() {
    for command in [
        vec!["schema"],
        vec!["capabilities"],
        vec!["doctor", "--offline"],
    ] {
        let result = Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
            .args(command)
            .assert()
            .success();
        serde_json::from_slice::<Value>(&result.get_output().stdout).unwrap();
    }
    Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
        .args(["notebooks", "list", "--fields", "typo"])
        .assert()
        .code(2);
    Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
        .args(["schema", "--command", "missing"])
        .assert()
        .code(4);
}

#[test]
fn write_previews_validate_content_without_a_desktop_connection() {
    for operation in ["create", "append"] {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("onenote"));
        cmd.args(["pages", operation]);
        if operation == "create" {
            cmd.args(["--section", "section", "--title", "Notes <&> 日本語"]);
        } else {
            cmd.arg("page");
        }
        let result = cmd
            .args([
                "--text",
                "Literal <script> & $(exit) ]]> 日本語",
                "--dry-run",
                "-ojson",
            ])
            .assert()
            .success();
        let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["operation"], operation);
        assert!(value["text"].as_str().unwrap().contains("<script>"));
    }
    let result = Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
        .args(["pages", "append", "page", "--file", "-", "--dry-run"])
        .write_stdin("\u{feff}Line one\r\n日本語\rEnd")
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(value["text"], "Line one\n日本語\nEnd");
    for args in [
        vec!["pages", "append", "page", "--dry-run"],
        vec!["pages", "append", "page", "--text", "", "--dry-run"],
        vec![
            "pages",
            "append",
            "page",
            "--text",
            "hi",
            "--file",
            "-",
            "--dry-run",
        ],
        vec![
            "pages",
            "create",
            "--section",
            "section",
            "--title",
            "\n",
            "--text",
            "hi",
            "--dry-run",
        ],
    ] {
        let result = Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
            .args(args)
            .assert()
            .code(2);
        assert!(result.get_output().stdout.is_empty());
    }
    for bytes in [vec![0xff], vec![b'x'; onenote_cli::write::MAX_TEXT + 1]] {
        Command::new(assert_cmd::cargo::cargo_bin!("onenote"))
            .args(["pages", "append", "page", "--file", "-", "--dry-run"])
            .write_stdin(bytes)
            .assert()
            .code(2);
    }
    for text in ["\u{ffff}", "\0", "\x1b"] {
        assert!(onenote_cli::write::validate(text, false).is_err());
    }
    let contract = schema::generate(None);
    for name in ["pages create", "pages append"] {
        let command = contract["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        assert_eq!(command["effects"], "non_idempotent");
    }
}

#[test]
fn locked_section_is_not_reported_as_an_empty_page_list() {
    let xml = r#"<one:Section xmlns:one="http://schemas.microsoft.com/office/onenote/2013/onenote" ID="locked" name="Protected" encrypted="true" locked="true"/>"#;
    for result in [
        model::collection(xml, "Page", &args(25, 0)),
        model::search(xml, &args(25, 0)),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.kind, "desktop_error");
        assert!(error.message.contains("Unlock it in OneNote desktop"));
    }
    assert_eq!(
        model::collection(xml, "Section", &args(25, 0)).unwrap()["items"][0]["locked"],
        true
    );
    let unlocked = xml.replace(" locked=\"true\"", "");
    assert_eq!(
        model::collection(&unlocked, "Page", &args(25, 0)).unwrap()["total"],
        0
    );
}

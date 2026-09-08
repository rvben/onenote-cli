use crate::{cli::Format, error::Error};
use serde_json::Value;
use std::io::{self, IsTerminal, Write};

pub fn structured(format: Format) -> bool {
    format == Format::Json || (format == Format::Auto && !io::stdout().is_terminal())
}
// Honor the last output selection even when clap cannot parse the command.
pub fn format_from_args(args: &[String]) -> Format {
    let mut format = Format::Auto;
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            break;
        }
        let value = if arg == "--output" || arg == "-o" {
            index += 1;
            args.get(index).map(String::as_str)
        } else {
            arg.strip_prefix("--output=")
                .or_else(|| arg.strip_prefix("-o="))
                .or_else(|| arg.strip_prefix("-o").filter(|s| !s.is_empty()))
        };
        if let Some(value) = value {
            format = match value {
                "json" => Format::Json,
                "text" => Format::Text,
                "auto" => Format::Auto,
                _ => format,
            };
        }
        index += 1;
    }
    format
}
pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| {
            (!c.is_control() || *c == '\n' || *c == '\t')
                && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}
pub fn render(value: &Value, color: bool) -> String {
    let title = |s: &str| {
        if color {
            format!("\x1b[1;35m{}\x1b[0m", clean(s))
        } else {
            clean(s)
        }
    };
    if let Some(items) = value["items"].as_array() {
        let mut text = format!(
            "{}\n",
            title(&format!("OneNote · {} of {}", items.len(), value["total"]))
        );
        if items.is_empty() {
            text.push_str("\n  No matching items.\n");
        }
        for item in items {
            text.push_str(&format!(
                "\n  {}\n",
                title(item["name"].as_str().unwrap_or("Item"))
            ));
            for field in ["path", "id", "last_modified", "locked", "kind", "parent_id"] {
                if let Some(v) = item.get(field).filter(|v| !v.is_null()) {
                    let v = v
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| v.to_string());
                    text.push_str(&format!("  {field}: {}\n", clean(&v)));
                }
            }
        }
        if !value["next_offset"].is_null() {
            text.push_str(&format!(
                "\n  Continue with the same command and --offset {}\n",
                value["next_offset"]
            ));
        }
        if value["indexing_pending"] == true {
            text.push_str(&format!("\n  {} unindexed pages omitted. Search results may be incomplete until Windows Search finishes indexing.\n", value["unindexed_count"]));
        }
        return text;
    }
    if let Some(markdown) = value["markdown"].as_str() {
        let mut text = format!(
            "{}\nid: {}\n\n{}\n",
            title(value["name"].as_str().unwrap_or("Untitled")),
            clean(value["id"].as_str().unwrap_or("")),
            clean(markdown)
        );
        if value["has_images"] == true || value["has_attachments"] == true {
            text.push_str("\nThis page also contains images or attachments; these are not included in the text.\n");
        }
        if let Some(xml) = value["xml"].as_str() {
            text.push_str(&format!("\n{}\n{}\n", title("Page XML"), clean(xml)));
        }
        return text;
    }
    if value.get("verified").is_some() {
        return format!(
            "{}\n{}\n",
            title(if value["available"] == true {
                "OneNote desktop is ready"
            } else {
                "OneNote desktop check"
            }),
            clean(value["message"].as_str().unwrap_or(""))
        );
    }
    clean(&serde_json::to_string_pretty(value).unwrap_or_default()) + "\n"
}
pub fn print(value: &Value, format: Format, no_color: bool) -> Result<(), Error> {
    let text = if structured(format) {
        serde_json::to_string_pretty(value).map_err(|e| Error::desktop(e.to_string()))? + "\n"
    } else {
        render(
            value,
            !no_color && std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal(),
        )
    };
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.map_err(Error::from),
    }
}
pub fn error(error: &Error, format: Format) {
    let text = if structured(format) {
        serde_json::json!({"error":{"kind":error.kind,"message":error.message,"exit_code":error.code(),"retryable":false}}).to_string()
    } else {
        format!("error: {}", clean(&error.message))
    };
    let _ = writeln!(io::stderr().lock(), "{text}");
}

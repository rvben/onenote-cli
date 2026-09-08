use crate::{cli::ContentArgs, error::Error};
use serde_json::{Value, json};
use std::io::{IsTerminal, Read};

pub const MAX_TEXT: usize = 1024 * 1024;

pub fn validate(text: &str, title: bool) -> Result<(), Error> {
    let max = if title { 512 } else { MAX_TEXT };
    if text.trim().is_empty() || text.len() > max {
        return Err(Error::invalid(format!(
            "{} must contain 1–{max} bytes of text",
            if title { "Title" } else { "Content" }
        )));
    }
    if text.chars().any(|c| {
        matches!(c, '\u{fffe}' | '\u{ffff}')
            || (c.is_control() && (title || !matches!(c, '\n' | '\r' | '\t')))
    }) {
        return Err(Error::invalid(
            "Text contains unsupported control or XML characters",
        ));
    }
    Ok(())
}

pub fn content(args: &ContentArgs) -> Result<String, Error> {
    let text = if let Some(text) = &args.text {
        text.clone()
    } else {
        let path = args
            .file
            .as_ref()
            .ok_or_else(|| Error::invalid("Use --text or --file"))?;
        let reader: Box<dyn Read> = if path.as_os_str() == "-" {
            if std::io::stdin().is_terminal() {
                return Err(Error::invalid("Pipe UTF-8 text into --file -"));
            }
            Box::new(std::io::stdin())
        } else {
            Box::new(std::fs::File::open(path)?)
        };
        let mut bytes = Vec::new();
        reader.take((MAX_TEXT + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_TEXT {
            return Err(Error::invalid("Content exceeds 1 MiB"));
        }
        String::from_utf8(bytes).map_err(|_| Error::invalid("Content must be UTF-8"))?
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    validate(text, false)?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

pub fn prepare(
    operation: &str,
    target: &str,
    title: Option<&str>,
    args: &ContentArgs,
) -> Result<Value, Error> {
    if let Some(title) = title {
        validate(title, true)?;
    }
    let text = content(args)?;
    Ok(json!({"operation":operation,"target":target,"title":title,"text":text}))
}

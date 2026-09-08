use crate::{cli::PageArgs, error::Error};
use roxmltree::{Document, Node};
use serde_json::{Value, json};

const NS: &str = "http://schemas.microsoft.com/office/onenote/2013/onenote";
pub const FIELDS: &[&str] = &[
    "id",
    "name",
    "kind",
    "parent_id",
    "path",
    "last_modified",
    "locked",
];

fn parse(xml: &str) -> Result<Document<'_>, Error> {
    let doc =
        Document::parse(xml).map_err(|e| Error::desktop(format!("Invalid OneNote XML: {e}")))?;
    if doc.root_element().tag_name().namespace() != Some(NS) {
        return Err(Error::desktop("Unexpected OneNote XML namespace"));
    }
    Ok(doc)
}
fn is(node: Node<'_, '_>, name: &str) -> bool {
    node.has_tag_name((NS, name))
}
fn hidden(node: Node<'_, '_>) -> bool {
    node.ancestors().any(|n| {
        n.attribute("isRecycleBin") == Some("true") || n.attribute("isInRecycleBin") == Some("true")
    })
}
fn record(node: Node<'_, '_>) -> Result<Value, Error> {
    let id = node
        .attribute("ID")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| Error::desktop("OneNote returned a resource without an ID"))?;
    let path: Vec<_> = node
        .ancestors()
        .filter(|n| n.is_element())
        .filter_map(|n| n.attribute("name"))
        .collect();
    Ok(json!({
        "id": id, "name": node.attribute("name").unwrap_or("Untitled"),
        "kind": node.tag_name().name().to_ascii_lowercase(),
        "parent_id": node.parent_element().and_then(|n| n.attribute("ID")),
        "path": path.into_iter().rev().collect::<Vec<_>>().join(" / "),
        "last_modified": node.attribute("lastModifiedTime"),
        "locked": node.ancestors().any(|n| n.attribute("locked") == Some("true")),
    }))
}
pub fn validate_fields(page: &PageArgs) -> Result<(), Error> {
    for field in &page.fields {
        if !FIELDS.contains(&field.as_str()) {
            return Err(Error::invalid(format!(
                "Unknown field '{field}'; choose from {}",
                FIELDS.join(", ")
            )));
        }
    }
    Ok(())
}
pub fn collection(xml: &str, kind: &str, page: &PageArgs) -> Result<Value, Error> {
    collect(xml, kind, page, false)
}
pub fn search(xml: &str, page: &PageArgs) -> Result<Value, Error> {
    collect(xml, "Page", page, true)
}
fn collect(xml: &str, kind: &str, page: &PageArgs, search: bool) -> Result<Value, Error> {
    validate_fields(page)?;
    let doc = parse(xml)?;
    if kind == "Page" && doc.root_element().attribute("locked") == Some("true") {
        return Err(Error::desktop(
            "This section is password-protected. Unlock it in OneNote desktop and retry.",
        ));
    }
    let mut nodes: Vec<_> = doc
        .descendants()
        .filter(|n| is(*n, kind) && !hidden(*n))
        .collect();
    // FindPages includes every unindexed page, even when it does not match.
    // Keep that information as completeness metadata, never as a search hit.
    let unindexed_count = if search {
        let before = nodes.len();
        nodes.retain(|node| node.attribute("isIndexed") != Some("false"));
        before - nodes.len()
    } else {
        0
    };
    let total = nodes.len();
    let start = page.offset as usize;
    let mut items = Vec::new();
    for node in nodes.into_iter().skip(start).take(page.limit as usize) {
        let mut item = record(node)?;
        if !page.fields.is_empty() {
            item.as_object_mut()
                .unwrap()
                .retain(|k, _| page.fields.contains(k));
        }
        items.push(item);
    }
    let next = start + items.len();
    let mut result = json!({"items":items, "total":total, "next_offset": if next < total { Some(next) } else { None }, "truncated":next < total});
    if search {
        result["indexing_pending"] = json!(unindexed_count > 0);
        result["unindexed_count"] = json!(unindexed_count);
    }
    Ok(result)
}
pub fn read(xml: &str, expected_id: &str, include_xml: bool) -> Result<Value, Error> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    if !is(root, "Page") || root.attribute("ID") != Some(expected_id) {
        return Err(Error::desktop(
            "OneNote returned a different page than requested",
        ));
    }
    let mut result = record(root)?;
    let mut fragments = Vec::new();
    for node in root.descendants().filter(|n| is(*n, "T")) {
        if node.ancestors().any(|n| is(n, "Title")) {
            continue;
        }
        let html: String = node.children().filter_map(|n| n.text()).collect();
        let rendered = html2md::parse_html(&html);
        if !rendered.trim().is_empty() {
            fragments.push(rendered.trim().to_owned());
        }
    }
    result["markdown"] = json!(fragments.join("\n\n"));
    result["has_images"] = json!(root.descendants().any(|n| is(n, "Image")));
    result["has_attachments"] = json!(root.descendants().any(|n| is(n, "InsertedFile")));
    if include_xml {
        result["xml"] = json!(xml);
    }
    Ok(result)
}

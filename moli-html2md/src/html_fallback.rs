use std::collections::HashSet;

use crate::{Dom, NodeKind};

/// Markdown has no merged cells, nested tables, or block content inside cells.
/// Detect those structures before conversion so a table is traversed at most
/// twice, even when it contains thousands of nested tables.
pub(crate) fn needed<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    let mut stack = vec![(dom.first_child(root), depth + 1)];
    while let Some((node, depth)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        stack.push((dom.next_sibling(node), depth));
        if let NodeKind::Element(tag) = dom.node_kind(node) {
            if matches!(
                tag,
                "table"
                    | "pre"
                    | "blockquote"
                    | "ul"
                    | "ol"
                    | "li"
                    | "hr"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            ) {
                return true;
            }
            if matches!(tag, "td" | "th")
                && ["rowspan", "colspan"].iter().any(|attr| {
                    dom.attribute(node, attr)
                        .is_some_and(|value| crate::table::has_cell_span(attr, value))
                })
            {
                return true;
            }
        }
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

/// Examine this table's own cells, not a data table nested inside a layout.
pub(crate) fn has_data_semantics<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    if dom.attribute(root, "role").is_some_and(|role| {
        matches!(
            role.trim().to_ascii_lowercase().as_str(),
            "table" | "grid" | "treegrid"
        )
    }) {
        return true;
    }
    let mut stack = vec![(dom.first_child(root), depth + 1)];
    while let Some((node, depth)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        stack.push((dom.next_sibling(node), depth));
        if let NodeKind::Element(tag) = dom.node_kind(node) {
            if tag == "table" {
                continue;
            }
            if matches!(tag, "th" | "thead" | "caption") {
                return true;
            }
            if matches!(tag, "td" | "th")
                && (dom.attribute(node, "scope").is_some()
                    || dom.attribute(node, "headers").is_some()
                    || ["rowspan", "colspan"].into_iter().any(|name| {
                        dom.attribute(node, name)
                            .is_some_and(|value| crate::table::has_cell_span(name, value))
                    }))
            {
                return true;
            }
        }
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

pub(crate) fn has_cells<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
) -> bool {
    let mut stack = vec![(dom.first_child(root), depth + 1)];
    while let Some((node, depth)) = stack.pop() {
        let Some(node) = node else {
            continue;
        };
        if depth >= limit {
            continue;
        }
        if matches!(dom.node_kind(node), NodeKind::Element("th" | "td")) {
            return true;
        }
        stack.push((dom.next_sibling(node), depth));
        stack.push((dom.first_child(node), depth + 1));
    }
    false
}

enum Task<Id> {
    Node(Id, usize, bool),
    Siblings(Option<Id>, usize, bool),
    Close(String),
    Fallback(String, usize),
    Literal(&'static str),
}

/// A compact HTML block is valid Markdown and preserves the original cell
/// associations. Keep content markup only; scripts, styles, event handlers
/// and unrelated presentation attributes are never copied.
pub(crate) fn render<D: Dom + ?Sized>(
    dom: &D,
    root: D::NodeId,
    depth: usize,
    limit: usize,
    anchor_targets: &crate::anchors::Targets,
) -> String {
    let keep_source_lines = matches!(dom.node_kind(root), NodeKind::Element("pre"));
    let mut output = String::new();
    let mut emitted_anchors = HashSet::new();
    let mut tasks = vec![Task::Node(root, depth, false)];
    while let Some(task) = tasks.pop() {
        match task {
            Task::Literal(value) => output.push_str(value),
            Task::Fallback(label, before) => {
                if output[before..]
                    .trim_matches(crate::writer::is_space)
                    .is_empty()
                {
                    output.push(' ');
                    escape(&label, &mut output);
                    output.push(' ');
                }
            }
            Task::Close(tag) => {
                if matches!(tag.as_str(), "table" | "thead" | "tbody" | "tfoot" | "tr") {
                    begin_html_line(&mut output);
                }
                output.push_str("</");
                output.push_str(&tag);
                output.push('>');
            }
            Task::Siblings(None, _, _) => {}
            Task::Siblings(Some(node), depth, code) => {
                tasks.push(Task::Siblings(dom.next_sibling(node), depth, code));
                tasks.push(Task::Node(node, depth, code));
            }
            Task::Node(node, depth, code) => {
                if depth >= limit {
                    continue;
                }
                if code && crate::content::is_code_control(dom, node) {
                    continue;
                }
                let code = code || matches!(dom.node_kind(node), NodeKind::Element("pre" | "code"));
                if let NodeKind::Element(tag) = dom.node_kind(node) {
                    if dom.has_block_layout(node) && !has_block_tag(tag) {
                        output.push_str("<div>");
                        tasks.push(Task::Close("div".to_owned()));
                    } else if dom.has_text_boundary(node) {
                        output.push(' ');
                        tasks.push(Task::Literal(" "));
                    }
                }
                if let NodeKind::Element(tag) = dom.node_kind(node)
                    && !allowed(tag)
                {
                    for id in anchor_targets.targets(dom, node) {
                        if emitted_anchors.insert(id.to_owned()) {
                            output.push_str(&crate::anchors::markup(id));
                        }
                    }
                }
                match dom.node_kind(node) {
                    NodeKind::Text(text) => {
                        escape_text(text, &mut output, code, keep_source_lines && code)
                    }
                    NodeKind::Element(
                        "head" | "title" | "script" | "style" | "noscript" | "template",
                    )
                    | NodeKind::Other => {}
                    NodeKind::Element("math") => {
                        for id in crate::anchors::within(dom, node, limit - depth, anchor_targets) {
                            if emitted_anchors.insert(id.clone()) {
                                output.push_str(&crate::anchors::markup(&id));
                            }
                        }
                        output.push_str(&crate::mathml::render(dom, node, limit - depth))
                    }
                    NodeKind::Element("input") => {
                        if let Some(value) = crate::form::input_text(dom, node) {
                            escape(&value, &mut output);
                        }
                    }
                    NodeKind::Element("textarea") => {
                        if let Some(value) = crate::form::textarea_text(dom, node) {
                            escape(&value, &mut output);
                        } else {
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code));
                        }
                    }
                    NodeKind::Element(tag) => {
                        if crate::visibility::nonrendered_serialized_state(dom, node) {
                            continue;
                        }
                        if matches!(tag, "iframe" | "video" | "audio") {
                            let urls = crate::media::sources(dom, node, tag, depth, limit);
                            if !urls.is_empty() {
                                output.push_str("<div>");
                                if tag == "video"
                                    && let Some(poster) =
                                        dom.attribute(node, "poster").filter(|src| {
                                            !src.trim().is_empty()
                                                && crate::media::safe_url(src, true)
                                        })
                                {
                                    output.push_str("<img src=\"");
                                    escape(poster, &mut output);
                                    output.push_str("\" alt=\"");
                                    escape(&crate::media::poster_label(dom, node), &mut output);
                                    output.push_str("\">");
                                }
                                for url in urls
                                    .into_iter()
                                    .filter(|url| crate::media::safe_url(url, false))
                                {
                                    output.push_str("<a href=\"");
                                    escape(url, &mut output);
                                    output.push_str("\">");
                                    escape(crate::media::label(dom, node, tag), &mut output);
                                    output.push_str("</a><br>");
                                }
                                output.push_str("</div>");
                                continue;
                            }
                        }
                        let retained = if allowed(tag)
                            && (tag != "span"
                                || dom.attribute(node, "id").is_some()
                                || dom.attribute(node, "title").is_some())
                        {
                            Some(tag)
                        } else {
                            None
                        };
                        if let Some(tag) = retained {
                            if matches!(
                                tag,
                                "table"
                                    | "thead"
                                    | "tbody"
                                    | "tfoot"
                                    | "tr"
                                    | "td"
                                    | "th"
                                    | "caption"
                            ) {
                                begin_html_line(&mut output);
                            }
                            output.push('<');
                            output.push_str(tag);
                            for name in [
                                "rowspan", "colspan", "scope", "headers", "id", "href", "src",
                                "alt", "title", "start", "reversed", "value", "label", "selected",
                                "multiple", "disabled", "name",
                            ] {
                                let image_source = if name == "src" && tag == "img" {
                                    crate::media::source(dom, node)
                                } else {
                                    None
                                };
                                let value = if name == "src" && tag == "img" {
                                    image_source.as_deref()
                                } else {
                                    dom.attribute(node, name)
                                };
                                if let Some(value) = value {
                                    if (name == "href" && !crate::media::safe_link_url(value))
                                        || (name == "src" && !crate::media::safe_url(value, true))
                                    {
                                        continue;
                                    }
                                    output.push(' ');
                                    output.push_str(name);
                                    output.push_str("=\"");
                                    escape(value, &mut output);
                                    output.push('"');
                                }
                            }
                            if matches!(tag, "pre" | "code")
                                && let Some(language) =
                                    dom.attribute(node, "class").and_then(|class| {
                                        class
                                            .split_ascii_whitespace()
                                            .find(|name| name.starts_with("language-"))
                                    })
                            {
                                output.push_str(" class=\"");
                                escape(language, &mut output);
                                output.push('"');
                            }
                            output.push('>');
                            if !matches!(tag, "br" | "hr" | "img") {
                                tasks.push(Task::Close(tag.to_owned()));
                            }
                        }
                        if let Some((math, relative_depth)) =
                            crate::mathml::primary_alternative(dom, node, limit - depth)
                        {
                            for id in
                                crate::anchors::within(dom, node, limit - depth, anchor_targets)
                            {
                                if emitted_anchors.insert(id.clone()) {
                                    output.push_str(&crate::anchors::markup(&id));
                                }
                            }
                            output.push_str(&crate::mathml::render(
                                dom,
                                math,
                                limit - depth - relative_depth,
                            ));
                        } else {
                            if let Some(label) = crate::content::fallback_text(dom, node) {
                                tasks.push(Task::Fallback(label, output.len()));
                            }
                            tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code));
                        }
                    }
                    NodeKind::Document => {
                        tasks.push(Task::Siblings(dom.first_child(node), depth + 1, code))
                    }
                }
            }
        }
    }
    output
}

fn allowed(tag: &str) -> bool {
    matches!(
        tag,
        "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "select"
            | "optgroup"
            | "option"
            | "button"
            | "textarea"
            | "p"
            | "div"
            | "span"
            | "a"
            | "img"
            | "br"
            | "hr"
            | "strong"
            | "b"
            | "em"
            | "i"
            | "s"
            | "del"
            | "sup"
            | "sub"
            | "code"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "dt"
            | "dd"
            | "details"
            | "summary"
    )
}

fn escape(text: &str, output: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            // Raw HTML blocks end at an empty Markdown line. Entities preserve
            // preformatted line endings without prematurely ending the block.
            '\n' => output.push_str("&#10;"),
            '\r' => output.push_str("&#13;"),
            _ => output.push(ch),
        }
    }
}

fn has_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "table"
            | "thead"
            | "tbody"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "caption"
            | "p"
            | "div"
            | "pre"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "dl"
            | "dt"
            | "dd"
            | "details"
            | "summary"
            | "hr"
    )
}

fn escape_text(text: &str, output: &mut String, code: bool, keep_newlines: bool) {
    for ch in text.chars() {
        if !code && matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{000c}') {
            if !output.ends_with([' ', '\n']) {
                output.push(' ');
            }
            continue;
        }
        if keep_newlines && matches!(ch, '\n' | '\r') {
            output.push(ch);
        } else if code && matches!(ch, '\\' | '*' | '_' | '`' | '~' | '[' | ']') {
            output.push_str(&format!("&#{};", ch as u32));
        } else {
            let mut bytes = [0; 4];
            escape(ch.encode_utf8(&mut bytes), output);
        }
    }
}

fn begin_html_line(output: &mut String) {
    while output.ends_with(' ') {
        output.pop();
    }
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
}

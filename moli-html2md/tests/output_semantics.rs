mod support;

use moli_html2md::{Dom, convert};
use support::{Tree, rendered_html};

fn containers(content: &str) -> [String; 3] {
    [
        format!("<div>{content}</div>"),
        format!("<table><tr><td>{content}</td><td>Other</td></tr></table>"),
        format!("<table><tr><td colspan='2'>{content}</td></tr></table>"),
    ]
}

fn descendants(tree: &Tree, root: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        found.push(node);
        let mut children = Vec::new();
        let mut child = tree.first_child(node);
        while let Some(node) = child {
            children.push(node);
            child = tree.next_sibling(node);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

fn text(tree: &Tree, root: usize) -> String {
    descendants(tree, root)
        .into_iter()
        .filter_map(|node| tree.nodes[node].text.as_deref())
        .collect()
}

fn with_attribute(tree: &Tree, name: &str, value: &str) -> Vec<usize> {
    descendants(tree, tree.root)
        .into_iter()
        .filter(|&node| tree.attribute(node, name) == Some(value))
        .collect()
}

#[test]
fn descendant_targets_preserve_author_link_ownership() {
    for tag in ["span", "kbd"] {
        let content =
            format!("<a href='#x'>Jump</a><a href='/doc'><{tag} id='x'>linked text</{tag}></a>");
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/doc");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            assert_eq!(text(&output, links[0]), "linked text", "{markdown}");
            let targets = with_attribute(&output, "id", "x");
            assert_eq!(targets.len(), 1, "{markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
        }
    }
}

#[test]
fn media_sources_keep_the_author_link_around_its_text() {
    for tag in ["video", "audio", "iframe"] {
        let content = format!("<a href='/watch'><{tag} src='/resource'></{tag}>Watch</a>");
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/watch");
            assert!(
                links
                    .iter()
                    .any(|&node| text(&output, node).contains("Watch")),
                "{source}: {markdown}"
            );
            assert!(
                links
                    .iter()
                    .any(|&node| text(&output, node).contains("/resource")),
                "{source}: {markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/resource").is_empty(),
                "{markdown}"
            );
        }
    }
}

fn block_text(tree: &Tree, root: usize) -> String {
    let mut result = String::new();
    for node in descendants(tree, root) {
        if tree.nodes[node].tag.as_deref().is_some_and(|tag| {
            matches!(
                tag,
                "p" | "div"
                    | "table"
                    | "tr"
                    | "td"
                    | "th"
                    | "br"
                    | "section"
                    | "article"
                    | "aside"
                    | "main"
                    | "nav"
                    | "header"
                    | "footer"
                    | "figure"
                    | "figcaption"
                    | "form"
                    | "fieldset"
                    | "legend"
                    | "address"
            )
        }) {
            result.push('\n');
        }
        if let Some(text) = &tree.nodes[node].text {
            result.push_str(text);
        }
    }
    result
}

#[test]
fn authored_block_boundaries_survive_every_output_path() {
    for tag in [
        "section",
        "article",
        "aside",
        "main",
        "nav",
        "header",
        "footer",
        "figure",
        "figcaption",
        "form",
        "fieldset",
        "legend",
        "address",
    ] {
        for source in containers(&format!("<{tag}>A</{tag}><{tag}>B</{tag}>")) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let text = block_text(&output, output.root);
            let words: Vec<_> = text.split_whitespace().collect();
            assert!(
                words.windows(2).any(|words| words == ["A", "B"]),
                "{source}: {markdown}\n{text}"
            );
        }
    }
}

#[test]
fn empty_targets_and_formatting_do_not_hide_accessible_names() {
    for icon in [
        "<span id='icon'></span>",
        "<code><span id='icon'></span></code>",
        "<pre><code><span id='icon'></span></code></pre>",
        "<h2><span id='icon'></span></h2>",
        "<blockquote><span id='icon'></span></blockquote>",
        "<ul><li><span id='icon'></span></li></ul>",
        "<i class='fa fa-search'></i>",
        "<em><strong></strong></em>",
        "<sub></sub>",
        "<span>&nbsp;</span>",
    ] {
        for control in ["button", "a href='/search'"] {
            let tag = control.split_whitespace().next().unwrap();
            let content =
                format!("<a href='#icon'>Jump</a><{control} aria-label='Search'>{icon}</{tag}>");
            for source in containers(&content) {
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                assert_eq!(
                    text(&output, output.root).matches("Search").count(),
                    1,
                    "{source}: {markdown}"
                );
                if tag == "a" {
                    let links = with_attribute(&output, "href", "/search");
                    assert!(
                        links
                            .iter()
                            .any(|&node| text(&output, node).contains("Search")),
                        "{markdown}"
                    );
                }
            }
        }
    }
}

#[test]
fn meaningful_objects_still_suppress_redundant_accessible_names() {
    for object in [
        "<img src='/icon.svg' alt=''>",
        "<math><mi>x</mi></math>",
        "<video src='/clip'></video>",
    ] {
        for source in containers(&format!("<button aria-label='Search'>{object}</button>")) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            assert!(
                !text(&output, output.root).contains("Search"),
                "{source}: {markdown}"
            );
        }
    }
}

#[test]
fn explicit_header_references_keep_ids_on_actual_header_cells() {
    struct NoFragments(Tree);
    impl Dom for NoFragments {
        type NodeId = usize;
        fn node_kind(&self, node: usize) -> moli_html2md::NodeKind<'_> {
            self.0.node_kind(node)
        }
        fn first_child(&self, node: usize) -> Option<usize> {
            self.0.first_child(node)
        }
        fn next_sibling(&self, node: usize) -> Option<usize> {
            self.0.next_sibling(node)
        }
        fn attribute(&self, node: usize, name: &str) -> Option<&str> {
            self.0.attribute(node, name)
        }
        fn may_have_fragment_links(&self) -> bool {
            false
        }
    }
    for span in ["", " colspan='2'"] {
        let source = format!(
            "<table><tr><th id='group'{span}>Group</th><th id='detail'>Detail</th></tr>\
             <tr><td headers='group detail'>A</td><td headers='group'>B</td></tr></table>"
        );
        let tree = Tree::parse(&source);
        let hinted = NoFragments(tree.clone());
        for markdown in [convert(&tree, tree.root), convert(&hinted, hinted.0.root)] {
            let output = Tree::parse(&rendered_html(&markdown));
            let tables = with_attribute(&output, "id", "group");
            assert_eq!(tables.len(), 1, "{markdown}");
            for cell in output
                .nodes
                .iter()
                .filter(|node| node.tag.as_deref() == Some("td"))
            {
                let references = cell.attrs.iter().find(|(key, _)| key == "headers");
                assert!(references.is_some(), "{markdown}");
                for id in references.unwrap().1.split_ascii_whitespace() {
                    let targets = with_attribute(&output, "id", id);
                    assert_eq!(targets.len(), 1, "{markdown}");
                    assert_eq!(
                        output.nodes[targets[0]].tag.as_deref(),
                        Some("th"),
                        "{markdown}"
                    );
                }
            }
        }
    }
}

#[test]
fn wide_math_targets_keep_order_and_are_emitted_once() {
    use std::fmt::Write as _;

    let count = 2048;
    let mut source = String::new();
    for index in 0..count {
        write!(source, "<a href='#m{index}'>Term</a>").unwrap();
    }
    source.push_str("<math>");
    for index in 0..count {
        write!(source, "<mrow id='m{index}'><mi>x</mi></mrow>").unwrap();
    }
    source.push_str("<mrow id='m0'><mi>y</mi></mrow></math>");
    let input = Tree::parse(&source);
    let markdown = convert(&input, input.root);
    let output = Tree::parse(&rendered_html(&markdown));
    let targets: Vec<_> = descendants(&output, output.root)
        .into_iter()
        .filter_map(|node| output.attribute(node, "id"))
        .filter(|id| *id != "turndown-root")
        .collect();
    let expected: Vec<_> = (0..count).map(|index| format!("m{index}")).collect();
    assert_eq!(targets, expected);
}

#[test]
fn nested_presentational_tables_expand_without_affecting_inner_data_tables() {
    for role in ["presentation", "none", " PRESENTATION "] {
        for inner_table in [
            "",
            "<table><tr><th>Key</th><th>Value</th></tr><tr><td>C</td><td>D</td></tr></table>",
        ] {
            let content =
                format!("<table role='{role}'><tr><td>A</td><td>B{inner_table}</td></tr></table>");
            for (source, outer) in [
                (content.clone(), 0),
                (
                    format!("<table><tr><td colspan='2'>{content}</td></tr></table>"),
                    1,
                ),
            ] {
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                let tables = output
                    .nodes
                    .iter()
                    .filter(|node| node.tag.as_deref() == Some("table"))
                    .count();
                assert_eq!(
                    tables,
                    outer + usize::from(!inner_table.is_empty()),
                    "{source}: {markdown}"
                );
                let text = block_text(&output, output.root);
                let words: Vec<_> = text.split_whitespace().collect();
                assert!(
                    words.windows(2).any(|words| words == ["A", "B"]),
                    "{markdown}"
                );
            }
        }
    }
}

#[test]
fn gfm_section_targets_land_in_their_first_cell() {
    let source = "<a href='#first'>First</a><a href='#second'>Second</a><a href='#empty'>Empty</a>\
        <table><tbody id='first'><tr><td>A</td><td>B</td></tr></tbody>\
        <tbody id='second'><tr><td>C</td><td>D</td></tr></tbody><tbody id='empty'></tbody></table>";
    let input = Tree::parse(source);
    let markdown = convert(&input, input.root);
    assert!(markdown.contains("| --- |"), "{markdown}");
    let output = Tree::parse(&rendered_html(&markdown));
    for (id, expected_cell) in [("first", "A"), ("second", "C")] {
        let targets = with_attribute(&output, "id", id);
        assert_eq!(targets.len(), 1, "{markdown}");
        let cells: Vec<_> = descendants(&output, output.root)
            .into_iter()
            .filter(|&node| {
                output.nodes[node].tag.as_deref() == Some("td")
                    && descendants(&output, node).contains(&targets[0])
            })
            .collect();
        assert_eq!(cells.len(), 1, "{markdown}");
        assert_eq!(text(&output, cells[0]).trim(), expected_cell, "{markdown}");
    }
    assert_eq!(
        with_attribute(&output, "id", "empty").len(),
        1,
        "{markdown}"
    );
}

#[test]
fn input_values_keep_their_word_boundaries_in_every_output_path() {
    for (content, expected) in [
        ("<input value='10'><input value='20'>", vec!["10", "20"]),
        (
            "<label>Min<input value='10'></label><label>Max<input value='20'></label>",
            vec!["Min", "10", "Max", "20"],
        ),
    ] {
        for source in containers(content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let cell = descendants(&output, output.root)
                .into_iter()
                .find(|&node| output.nodes[node].tag.as_deref() == Some("td"))
                .unwrap_or(output.root);
            let content = text(&output, cell);
            assert_eq!(
                content.split_whitespace().collect::<Vec<_>>(),
                expected,
                "{source}: {markdown}"
            );
        }
    }
}

#[test]
fn retained_dialogs_and_details_preserve_their_open_state() {
    for tag in ["dialog", "details"] {
        for state in ["open", "open=''", "open='false'", ""] {
            let input = Tree::parse(&format!(
                "<table><tr><td colspan='2'><{tag} {state}>Important content</{tag}></td></tr></table>"
            ));
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let elements: Vec<_> = descendants(&output, output.root)
                .into_iter()
                .filter(|&node| output.nodes[node].tag.as_deref() == Some(tag))
                .collect();
            assert_eq!(elements.len(), 1, "{markdown}");
            assert_eq!(
                output.attribute(elements[0], "open").is_some(),
                !state.is_empty(),
                "{markdown}"
            );
            assert_eq!(
                text(&output, elements[0]),
                "Important content",
                "{markdown}"
            );
        }
    }
}

#[test]
fn inline_html_fallback_inherits_and_releases_the_writer_link_context() {
    for media in ["video", "audio", "iframe"] {
        let content = format!(
            "<a href='/watch'><code id='snippet'><{media} src='/v.mp4'></{media}></code>Watch</a>\
             <a href='#snippet'>Jump</a>\
             <code id='outside'><video src='/outside.mp4'></video></code><a href='#outside'>Outside</a>"
        );
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/watch");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            let body = text(&output, links[0]);
            assert!(
                body.contains("Watch") && body.contains("/v.mp4"),
                "{source}: {markdown}"
            );
            let targets = with_attribute(&output, "id", "snippet");
            assert_eq!(targets.len(), 1, "{source}: {markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/v.mp4").is_empty(),
                "{markdown}"
            );
            let outside = with_attribute(&output, "href", "/outside.mp4");
            assert_eq!(outside.len(), 1, "{markdown}");
            assert!(
                !descendants(&output, links[0]).contains(&outside[0]),
                "{markdown}"
            );
        }
    }
}

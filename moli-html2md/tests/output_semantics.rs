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

mod support;

use moli_html2md::convert;
use pulldown_cmark::{Event, Options, Parser};
use support::Tree;

fn markdown(source: &str) -> String {
    let tree = Tree::parse(source);
    convert(&tree, tree.root)
}

fn math_payloads(markdown: &str) -> Vec<String> {
    Parser::new_ext(markdown, Options::ENABLE_MATH)
        .filter_map(|event| match event {
            Event::InlineMath(value) | Event::DisplayMath(value) => Some(value.into_string()),
            _ => None,
        })
        .collect()
}

fn rendered_math_html(markdown: &str) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, Parser::new_ext(markdown, Options::ENABLE_MATH));
    html
}

#[test]
fn image_dollars_preserve_images_links_and_adjacent_formulas() {
    for (source, expected_math) in [
        (r#"<p><img src="/p.png" alt="$5">(\(x\))</p>"#, vec!["x"]),
        (
            r#"<p><a href="/details"><img src="/p.png" alt="$5"></a>\(x\)</p>"#,
            vec!["x"],
        ),
        (
            r#"<p>\(x\)<img src="/p.png" alt="$5"><a href="/details">details</a>\(y\)</p>"#,
            vec!["x", "y"],
        ),
    ] {
        let result = markdown(source);
        assert_eq!(math_payloads(&result), expected_math, "{source}: {result}");
        let html = rendered_math_html(&result);
        assert!(html.contains("<img src=\"/p.png\" alt=\"$5\""), "{html}");
        if source.contains("/details") {
            assert!(html.contains("<a href=\"/details\">"), "{html}");
        }
    }
}

#[test]
fn verbatim_dollars_fall_back_without_partial_math() {
    for expression in [
        r"\(\verb|$|\)",
        r"\[\verb|$|\]",
        r"\(\verb|a$b$c|\)",
        r"\(\verb$literal$\)",
        r"\(\verb|\\$|\)",
    ] {
        let result = markdown(&format!("<p>{expression}$x$</p>"));
        assert_eq!(math_payloads(&result), ["x"], "{expression}: {result}");
        let events: String = Parser::new_ext(&result, Options::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::Text(value) => Some(value.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(events, expression, "{result}");
    }
    for payload in [r"\verb|a  b|", r"\verb|\$|", r"\verb*|50%|"] {
        let result = markdown(&format!(r"<p>\({payload}\)</p>"));
        assert_eq!(math_payloads(&result), [payload], "{result}");
    }
}

#[test]
fn mathml_alternatives_preserve_links_and_inline_semantics() {
    for (open, close) in [
        ("<a href=\"/details\">", "</a>"),
        ("<sup>", "</sup>"),
        ("<sub>", "</sub>"),
        ("<sup><a href=\"/details\">", "</a></sup>"),
    ] {
        for hidden_first in [false, true] {
            let primary = format!("{open}<math><mi>x</mi></math>{close}");
            let hidden = "<span aria-hidden=\"true\">visual-only x</span>";
            let children = if hidden_first {
                format!("{hidden}{primary}")
            } else {
                format!("{primary}{hidden}")
            };
            let result = markdown(&format!("<span>{children}</span>"));
            let html = rendered_math_html(&result);
            assert!(html.contains(&primary), "{result}: {html}");
            assert_eq!(html.matches("<math>").count(), 1, "{html}");
            assert!(!html.contains("visual-only"), "{html}");
        }
    }
}

#[test]
fn mathml_actions_keep_the_first_branch_and_a_single_fraction_argument() {
    for (action, numerator) in [
        (
            "<maction><mn>1</mn><mn>2</mn></maction>",
            "<mrow><mn>1</mn></mrow>",
        ),
        (
            "<maction>ignored text<mn>1</mn><mn>2</mn></maction>",
            "<mrow><mn>1</mn></mrow>",
        ),
        ("<maction></maction>", "<mrow></mrow>"),
        (
            "<maction><mrow><mn>1</mn><mo>+</mo><mn>2</mn></mrow><mn>9</mn></maction>",
            "<mrow><mrow><mn>1</mn><mo>+</mo><mn>2</mn></mrow></mrow>",
        ),
    ] {
        let result = markdown(&format!("<math><mfrac>{action}<mn>3</mn></mfrac></math>"));
        let expected = format!("<math><mfrac>{numerator}<mn>3</mn></mfrac></math>");
        assert_eq!(result, expected, "{action}");
        assert!(rendered_math_html(&result).contains(&expected), "{result}");
        assert_eq!(rendered_math_html(&result), support::rendered_html(&result));
    }
}

#[test]
fn filtered_mathml_containers_keep_their_grouping() {
    let result =
        markdown("<math><mfrac><unknown><mn>1</mn><mn>2</mn></unknown><mn>3</mn></mfrac></math>");
    assert_eq!(
        result,
        "<math><mfrac><mrow><mn>1</mn><mn>2</mn></mrow><mn>3</mn></mfrac></math>"
    );
}

#[test]
fn tex_comments_preserve_text_and_control_word_boundaries() {
    for command in [
        "text",
        "mbox",
        "textrm",
        "textbf",
        "textit",
        "texttt",
        "textsf",
        "textnormal",
        "hbox",
    ] {
        for newline in ["\n", "\r\n", "\r"] {
            let result = markdown(&format!(
                "<p>\\(\\{command}{{foo% comment{newline}bar}}\\)</p>"
            ));
            assert_eq!(
                math_payloads(&result),
                [format!(r"\{command}{{foobar}}")],
                "{result}"
            );
        }
    }
    for (payload, expected) in [
        ("\\alpha% comment\nb", r"\alpha b"),
        ("\\alpha% first\n% second\nb", r"\alpha b"),
        ("\\alpha% first\nb% second\nc", r"\alpha bc"),
        ("\\\\alpha% comment\nb", r"\\alphab"),
        ("\\text{foo % comment\nbar}", r"\text{foo bar}"),
        (
            "\\text{foo% comment\n\\textbf{bar}}",
            r"\text{foo\textbf{bar}}",
        ),
    ] {
        let result = markdown(&format!(r"<p>\({payload}\)</p>"));
        assert_eq!(math_payloads(&result), [expected], "{result}");
    }
}

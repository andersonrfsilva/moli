use super::automatic_controls::font_usage_vm;
use super::selection_order::order_font_data;
use super::*;

#[test]
fn automatic_fonts_default_markers_use_rendered_list_types_and_counters() {
    let (_, wide) = order_font_data();
    for (markup, css, range, expected) in [
        (
            "<ol><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<ul><li class='target'>body</li></ul>",
            "",
            "U+2022",
            "loaded",
        ),
        (
            "<ol start='26' style='list-style-type:lower-alpha'><li class='target'>body</li></ol>",
            "",
            "U+007A",
            "loaded",
        ),
        (
            "<ol reversed><li class='target'>body</li><li>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol reversed start='5'><li>body</li><li class='target'>body</li></ol>",
            "",
            "U+0034",
            "loaded",
        ),
        (
            "<ol start='4'><li value='8'>body</li><li class='target'>body</li></ol>",
            "",
            "U+0039",
            "loaded",
        ),
        (
            "<ol start='-2'><li class='target'>body</li></ol>",
            "",
            "U+002D",
            "loaded",
        ),
        (
            "<ol><li hidden>body</li><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<ol><li style='content-visibility:hidden'>body</li><li class='target'>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol><li>outer<ol><li>nested</li></ol></li><li class='target'>body</li></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol><div style='display:contents'><li class='target'>body</li></div></ol>",
            "",
            "U+0031",
            "loaded",
        ),
        (
            "<div class='target' style='display:list-item;list-style-type:upper-alpha'>body</div>",
            "",
            "U+0041",
            "loaded",
        ),
        (
            "<ol><li>body</li><div class='target' value='8' style='display:list-item'>body</div></ol>",
            "",
            "U+0032",
            "loaded",
        ),
        (
            "<ol style='list-style-type:none'><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "unloaded",
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            ".target::marker{content:none}",
            "U+0031",
            "unloaded",
        ),
        (
            "<ol><li class='target'>body</li></ol>",
            ".target::marker{display:contents}",
            "U+0031",
            "loaded",
        ),
        (
            "<ol start='4'><li class='target'>body</li></ol>",
            "",
            "U+0031",
            "unloaded",
        ),
    ] {
        let mut vm = font_usage_vm();
        let markup = serde_json::to_string(markup).unwrap();
        let css = serde_json::to_string(&format!(".target::marker{{font-family:MarkerFace}}{css}"))
            .unwrap();
        vm.eval(&format!(r#"
            const style=document.createElement('style');style.textContent={css};document.head.append(style);
            document.body.innerHTML={markup};
            globalThis.face=new FontFace('MarkerFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'{range}'}});
            document.fonts.add(face);
        "#)).unwrap();
        vm.reconcile_document_font_resources();
        vm.eval("document.fonts.ready.then(()=>globalThis.ready=true)")
            .unwrap();
        run_font_tasks(&mut vm);
        assert_eq!(
            vm.eval("face.status").unwrap(),
            expected,
            "{markup}/{css}/{range}"
        );
        if css.contains("::marker{display:contents}") {
            assert_eq!(
                vm.eval("getComputedStyle(document.querySelector('.target'),'::marker').display")
                    .unwrap(),
                "inline"
            );
        }
        assert_eq!(vm.eval("globalThis.ready===true").unwrap(), "true");
        assert_eq!(vm.layout_pass_observability_for_test().1, 0);
    }
}

#[test]
fn automatic_fonts_default_markers_count_slotted_items_in_rendered_order() {
    let mut vm = font_usage_vm();
    let (_, wide) = order_font_data();
    vm.eval(&format!(r#"
        const style=document.createElement('style');
        style.textContent='.target::marker{{font-family:MarkerFace}}';document.head.append(style);
        const host=document.createElement('div');
        host.innerHTML='<li>first</li><li class="target" style="list-style-type:decimal">second</li>';
        const shadow=host.attachShadow({{mode:'open'}});
        shadow.innerHTML='<style>slot {{display:contents}}</style><ol start="8"><slot></slot></ol>';
        document.body.append(host);
        globalThis.face=new FontFace('MarkerFace','url(data:font/ttf;base64,{wide})',{{unicodeRange:'U+0039'}});
        document.fonts.add(face);
    "#)).unwrap();
    vm.reconcile_document_font_resources();
    run_font_tasks(&mut vm);
    assert_eq!(vm.eval("face.status").unwrap(), "loaded");
    assert_eq!(vm.layout_pass_observability_for_test().1, 0);
}

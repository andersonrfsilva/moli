//! Text/style resource discovery. This module never constructs layout boxes,
//! shapes text, queries geometry, paints, or starts a network operation.

use std::collections::{BTreeSet, HashMap, HashSet};

use moli_layout::{
    LayoutFormControlKind, LayoutInputControlKind, TextCharacter, TextTransform,
    TextTransformState, TextWhiteSpaceCollapse, classify_text_character, form_control_text,
};
use style::{
    computed_values::{list_style_position::T as MarkerPosition, position::T as Position},
    properties::ComputedValues,
    servo_arc::Arc,
    values::{
        computed::{Content, ContentItem, Float},
        specified::box_::{ContentVisibility, DisplayInside, DisplayOutside},
    },
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        element::{StyleObservation, text_control_value},
    },
};

use super::demand::{FontUse, FontUses};

enum Work {
    Node(DomHandle, Option<Arc<ComputedValues>>),
    Text(Arc<ComputedValues>, String),
    Generated(Arc<ComputedValues>, bool),
    Restore(TextContext),
}

pub(crate) fn collect_font_uses(
    host: &JsContextHost,
    document: DomHandle,
    families: &HashSet<String>,
) -> FontUses {
    let Some(root) = host
        .dom_host()
        .dom()
        .document_element_handle_for_document(document)
    else {
        return FontUses::new();
    };
    let mut observation = StyleObservation::new(host);
    let mut collector = TextCollector {
        families,
        queries: HashMap::new(),
        indices: HashMap::new(),
        uses: Vec::new(),
        context: TextContext::default(),
    };
    let mut pending = vec![Work::Node(root, None)];
    while let Some(work) = pending.pop() {
        let (node, inherited) = match work {
            Work::Restore(context) => {
                collector.context = context;
                continue;
            }
            Work::Text(style, text) => {
                collector.text(&style, &text);
                continue;
            }
            Work::Generated(style, separate) => {
                if style.clone_display().is_none()
                    || !matches!(&style.get_counters().content, Content::Items(_))
                {
                    continue;
                }
                let saved = collector.enter(&style, false, separate);
                collector.generated_text(&style);
                if let Some(context) = saved {
                    collector.context = context;
                }
                continue;
            }
            Work::Node(node, inherited) => (node, inherited),
        };
        let Some(native) = host.dom_host().node(node) else {
            continue;
        };
        if let Some(text) = native
            .data_value()
            .filter(|_| native.is_text() || native.is_cdata_section())
        {
            if let Some(style) = inherited {
                collector.text(&style, text);
            }
            continue;
        }
        let Some(element) = native.as_element() else {
            continue;
        };
        let read = observation.read(node);
        let Some(style) = read.computed_values() else {
            continue;
        };
        if style.clone_display().is_none()
            || style.clone_content_visibility() == ContentVisibility::Hidden
            || (host.document_scripting_enabled(document)
                && host.dom_host().is_html_element_named(node, "noscript"))
        {
            continue;
        }
        if host.dom_host().is_html_element_named(node, "br") {
            collector.context = TextContext::default();
            continue;
        }
        let control = if element.is_html_input() {
            let kind = LayoutInputControlKind::from_type_attribute(element.attribute("type"));
            if kind == LayoutInputControlKind::Hidden {
                continue;
            }
            Some(LayoutFormControlKind::Input(kind))
        } else if element.is_html_textarea() {
            Some(LayoutFormControlKind::TextArea)
        } else {
            None
        };
        let replaced = control.is_some()
            || matches!(
                element.local_name(),
                "img"
                    | "iframe"
                    | "frame"
                    | "canvas"
                    | "embed"
                    | "audio"
                    | "video"
                    | "svg"
                    | "select"
            );
        if let Some(context) = collector.enter(&style, replaced, false) {
            pending.push(Work::Restore(context));
        }
        if let Some(kind) = control {
            let (text, is_placeholder) = form_control_text(
                kind,
                &text_control_value(host, node),
                element.attribute("placeholder").unwrap_or(""),
            );
            let text_style = if is_placeholder {
                read.pseudo_computed_values("placeholder").unwrap_or(style)
            } else {
                style
            };
            pending.push(Work::Text(text_style, text));
            // A textarea's children are its default value, not its live value.
            continue;
        }
        let marker = style
            .clone_display()
            .is_list_item()
            .then(|| read.pseudo_computed_values("marker"))
            .flatten();
        let marker_separate = style.clone_list_style_position() == MarkerPosition::Outside;
        let (_, before, after) = read.into_element_computed_values().expect("observed style");
        if let Some(after) = after {
            pending.push(Work::Generated(after, false));
        }
        // Follow DOM slot assignment rather than an intermediate layout tree.
        let children = if let Some(shadow) = host.dom_host().shadow_root_handle(node) {
            host.dom_host().child_handles(shadow).collect::<Vec<_>>()
        } else if host.dom_host().is_html_element_named(node, "slot") {
            let assigned = host
                .dom_host()
                .assigned_nodes_for_slot_with_options(node, false);
            if assigned.is_empty() {
                host.dom_host().child_handles(node).collect()
            } else {
                assigned
            }
        } else {
            host.dom_host().child_handles(node).collect()
        };
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|child| Work::Node(child, Some(style.clone()))),
        );
        if let Some(before) = before {
            pending.push(Work::Generated(before, false));
        }
        if let Some(marker) = marker {
            pending.push(Work::Generated(marker, marker_separate));
        }
    }
    collector
        .uses
        .into_iter()
        .filter(|(_, characters)| !characters.is_empty())
        .collect()
}

#[derive(Default)]
struct TextContext {
    transform: TextTransformState,
    // Outer None means no pending whitespace; inner None is an irrelevant font.
    pending_space: Option<Option<usize>>,
    line_has_content: bool,
    carriage_return: bool,
}

struct TextCollector<'a> {
    families: &'a HashSet<String>,
    queries: HashMap<usize, (Arc<ComputedValues>, Option<usize>)>,
    indices: HashMap<FontUse, usize>,
    uses: Vec<(FontUse, BTreeSet<char>)>,
    context: TextContext,
}

impl TextCollector<'_> {
    fn enter(
        &mut self,
        style: &ComputedValues,
        replaced: bool,
        separate: bool,
    ) -> Option<TextContext> {
        if separate {
            return Some(std::mem::take(&mut self.context));
        }
        let display = style.clone_display();
        let out_of_flow = matches!(style.clone_position(), Position::Absolute | Position::Fixed);
        let floated = style.clone_float() != Float::None;
        if !replaced
            && !out_of_flow
            && !floated
            && (display.is_contents()
                || (display.outside() == DisplayOutside::Inline
                    && display.inside() == DisplayInside::Flow))
        {
            return None;
        }
        if !out_of_flow {
            if floated || display.outside() == DisplayOutside::Inline {
                self.flush_space();
                self.context.line_has_content = true;
            } else {
                self.context = TextContext::default();
            }
        }
        Some(std::mem::take(&mut self.context))
    }

    fn query(&mut self, style: &Arc<ComputedValues>) -> Option<usize> {
        let identity = std::ptr::from_ref(style.get_font()) as usize;
        if let Some((_, query)) = self.queries.get(&identity) {
            return *query;
        }
        let usage = FontUse::from_style(style);
        let relevant = usage
            .families
            .iter()
            .any(|name| self.families.contains(name));
        let query = relevant.then(|| {
            if let Some(index) = self.indices.get(&usage) {
                *index
            } else {
                let index = self.uses.len();
                self.indices.insert(usage.clone(), index);
                self.uses.push((usage, BTreeSet::new()));
                index
            }
        });
        // Retaining the style keeps its font-struct identity from being recycled.
        self.queries.insert(identity, (style.clone(), query));
        query
    }

    fn flush_space(&mut self) {
        if let Some(Some(index)) = self.context.pending_space.take()
            && self.context.line_has_content
        {
            self.uses[index].1.insert(' ');
        }
    }

    fn text(&mut self, style: &Arc<ComputedValues>, text: &str) {
        if text.is_empty() {
            return;
        }
        let query = self.query(style);
        let transform = TextTransform::from(style.clone_text_transform().case());
        let whitespace = TextWhiteSpaceCollapse::from(style.clone_white_space_collapse());
        // Even irrelevant fonts must advance the common inline word/space state.
        for source in text.chars() {
            if std::mem::take(&mut self.context.carriage_return) && source == '\n' {
                continue;
            }
            let source = if source == '\r' {
                self.context.carriage_return = true;
                '\n'
            } else {
                source
            };
            for character in self.context.transform.transform(source, transform) {
                match classify_text_character(character, whitespace) {
                    TextCharacter::Collapsible { .. } => {
                        self.context.pending_space.get_or_insert(query);
                    }
                    TextCharacter::ForcedBreak { preserve_pending } => {
                        if preserve_pending {
                            self.flush_space();
                        }
                        self.context.pending_space = None;
                        self.context.line_has_content = false;
                        self.context.transform.reset();
                    }
                    TextCharacter::Preserved { character, .. } => {
                        self.flush_space();
                        if let Some(index) = query
                            && !character.is_control()
                        {
                            self.uses[index].1.insert(character);
                        }
                        self.context.line_has_content = true;
                    }
                }
            }
        }
    }

    fn generated_text(&mut self, style: &Arc<ComputedValues>) {
        if let Content::Items(items) = &style.get_counters().content {
            for item in &items.items[..items.alt_start] {
                if let ContentItem::String(text) = item {
                    self.text(style, text);
                }
            }
        }
    }
}

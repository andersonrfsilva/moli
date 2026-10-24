//! Text/style resource discovery. This module never constructs layout boxes,
//! shapes text, queries geometry, paints, or starts a network operation.

use std::collections::{HashMap, HashSet};

use style::{
    properties::ComputedValues,
    servo_arc::Arc,
    values::computed::{Content, ContentItem},
    values::specified::{box_::ContentVisibility, text::TextTransformCase},
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        element::{StyleObservation, text_control_value},
    },
};

use super::demand::{FontUse, FontUses};

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
        uses: FontUses::new(),
    };
    let mut pending = vec![(root, None::<Arc<ComputedValues>>)];
    while let Some((node, inherited)) = pending.pop() {
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
        if !native.is_element() {
            continue;
        }
        let Some((style, before, after)) = observation.read(node).into_element_computed_values()
        else {
            continue;
        };
        if style.clone_display().is_none()
            || style.clone_content_visibility() == ContentVisibility::Hidden
            || (host.document_scripting_enabled(document)
                && host.dom_host().is_html_element_named(node, "noscript"))
        {
            continue;
        }
        for pseudo in before.iter().chain(after.iter()) {
            if !pseudo.clone_display().is_none() {
                collector.generated_text(pseudo);
            }
        }
        if matches!(
            native.as_element().map(|element| element.local_name()),
            Some("input" | "textarea")
        ) {
            let value = text_control_value(host, node);
            if !value.is_empty() {
                collector.text(&style, &value);
            } else if let Some(placeholder) = host.dom_host().get_attribute(node, "placeholder") {
                collector.text(&style, &placeholder);
            }
            // A textarea's child text is its default value, not its live value.
            continue;
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
                .map(|child| (child, Some(style.clone()))),
        );
    }
    collector.uses
}

struct TextCollector<'a> {
    families: &'a HashSet<String>,
    queries: HashMap<usize, (Arc<ComputedValues>, Option<FontUse>)>,
    uses: FontUses,
}

impl TextCollector<'_> {
    fn text(&mut self, style: &Arc<ComputedValues>, text: &str) {
        if text.is_empty() {
            return;
        }
        let identity = std::ptr::from_ref(style.get_font()) as usize;
        let query = self.queries.entry(identity).or_insert_with(|| {
            let usage = FontUse::from_style(style);
            let relevant = usage
                .families
                .iter()
                .any(|name| self.families.contains(name));
            // Retain the style so its font-struct pointer cannot be recycled
            // while this discovery is using it as an identity.
            (style.clone(), relevant.then_some(usage))
        });
        let Some(query) = &query.1 else {
            return;
        };
        let characters = self.uses.entry(query.clone()).or_default();
        match style.clone_text_transform().case() {
            TextTransformCase::Uppercase => {
                characters.extend(
                    text.chars()
                        .filter(|c| !c.is_control())
                        .flat_map(char::to_uppercase),
                );
            }
            TextTransformCase::Lowercase => {
                characters.extend(
                    text.chars()
                        .filter(|c| !c.is_control())
                        .flat_map(char::to_lowercase),
                );
            }
            TextTransformCase::Capitalize => {
                let mut boundary = true;
                for character in text.chars().filter(|c| !c.is_control()) {
                    if boundary && character.is_alphabetic() {
                        characters.extend(character.to_uppercase());
                    } else {
                        characters.insert(character);
                    }
                    boundary = !character.is_alphanumeric();
                }
            }
            _ => characters.extend(text.chars().filter(|c| !c.is_control())),
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

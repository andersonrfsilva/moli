use std::collections::HashMap;

use crate::{
    document_runtime::DomHandle,
    dom_presentation::{flat_children, flat_parent, is_list_container, list_data},
    native_bridge::{JsContextHost, element::StyleObservation},
};
use moli_layout::{LayoutInputControlKind, ListCounter};

#[derive(Default)]
pub(super) struct ListCounters {
    values: HashMap<DomHandle, i32>,
}

impl ListCounters {
    pub(super) fn value(
        &mut self,
        host: &JsContextHost,
        observation: &mut StyleObservation<'_>,
        root: DomHandle,
        item: DomHandle,
        scripting: bool,
    ) -> i32 {
        if let Some(value) = self.values.get(&item) {
            return *value;
        }
        // A list container's own list-item box is not one of an outer list's
        // items. Its descendants belong to its separate counter scope.
        if !host
            .dom_host()
            .node(item)
            .and_then(|node| node.as_element())
            .is_some_and(is_list_container)
        {
            let mut ancestor = flat_parent(host.dom_host(), root, item);
            while let Some(node) = ancestor {
                let Some(element) = host
                    .dom_host()
                    .node(node)
                    .and_then(|node| node.as_element())
                else {
                    break;
                };
                let Some(style) = observation.read(node).computed_values() else {
                    break;
                };
                if !style.clone_display().is_contents() {
                    if is_list_container(element) {
                        self.collect_scope(host, observation, root, node, scripting);
                        break;
                    }
                    // Container traversal stops at a list item; nested items
                    // without their own HTML list use the renderer's fallback.
                    if style.clone_display().is_list_item() {
                        break;
                    }
                }
                ancestor = flat_parent(host.dom_host(), root, node);
            }
        }
        *self.values.entry(item).or_insert(1)
    }

    fn collect_scope(
        &mut self,
        host: &JsContextHost,
        observation: &mut StyleObservation<'_>,
        root: DomHandle,
        container: DomHandle,
        scripting: bool,
    ) {
        let dom = host.dom_host();
        let data = dom
            .node(container)
            .and_then(|node| node.as_element())
            .and_then(list_data)
            .unwrap_or_default();
        let mut items = Vec::new();
        let mut pending = flat_children(dom, root, container);
        pending.reverse();
        while let Some(node) = pending.pop() {
            let Some(element) = dom.node(node).and_then(|node| node.as_element()) else {
                continue;
            };
            let Some(style) = observation.read(node).computed_values() else {
                continue;
            };
            let display = style.clone_display();
            if display.is_none()
                || (scripting && element.is_html_element("noscript"))
                || (element.is_html_input()
                    && LayoutInputControlKind::from_type_attribute(element.attribute("type"))
                        == LayoutInputControlKind::Hidden)
            {
                continue;
            }
            if !display.is_contents() {
                if is_list_container(element) {
                    continue;
                }
                if display.is_list_item() {
                    items.push((node, list_data(element).and_then(|data| data.value)));
                    continue;
                }
            }
            // Replaced controls and media have no ordinary rendered children.
            if element.is_html_input()
                || element.is_html_textarea()
                || element.is_html_select()
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
                        | "progress"
                        | "meter"
                )
            {
                continue;
            }
            pending.extend(flat_children(dom, root, node).into_iter().rev());
        }
        let mut counter = ListCounter::new(data, items.len());
        for (node, explicit_value) in items {
            self.values.insert(node, counter.next_value(explicit_value));
        }
    }
}

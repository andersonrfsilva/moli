//! DOM text and attribute projections shared by resource discovery and rendering.
//! These helpers do not build layout boxes or query geometry.

use moli_layout::{LayoutFormControlData, LayoutListData};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Element, Node},
};

pub(crate) fn form_control_data(element: &Element) -> LayoutFormControlData {
    let value = if element.is_html_input() || element.is_html_textarea() {
        element.input_value()
    } else {
        element.attribute("value").unwrap_or("").to_owned()
    };
    LayoutFormControlData {
        value: value.into(),
        placeholder: element.attribute("placeholder").unwrap_or("").into(),
        size: optional_positive_u16_attribute(element, "size", 1, 65_535),
        columns: optional_positive_u16_attribute(element, "cols", 1, 65_535).unwrap_or(20),
        rows: optional_positive_u16_attribute(element, "rows", 1, 65_535).unwrap_or(2),
        checked: element.checked(),
        disabled: element.attribute("disabled").is_some(),
        multiple: element.attribute("multiple").is_some(),
        ..LayoutFormControlData::default()
    }
}

pub(crate) fn is_list_container(element: &Element) -> bool {
    ["ol", "ul", "menu"]
        .iter()
        .any(|name| element.is_html_element(name))
}

pub(crate) fn list_data(element: &Element) -> Option<LayoutListData> {
    let container = is_list_container(element);
    let item = element.is_html_element("li");
    (container || item).then(|| LayoutListData {
        ordered: element.is_html_element("ol"),
        start: container
            .then(|| signed_i32_attribute(element, "start"))
            .flatten(),
        reversed: container && element.attribute("reversed").is_some(),
        value: item
            .then(|| signed_i32_attribute(element, "value"))
            .flatten(),
    })
}

fn signed_i32_attribute(element: &Element, name: &str) -> Option<i32> {
    element.attribute(name)?.trim().parse::<i32>().ok()
}

pub(crate) fn select_control_data(
    host: &DomHost,
    node: DomHandle,
    element: &Element,
) -> LayoutFormControlData {
    let option_text = |option| {
        host.node(option)
            .and_then(Node::as_element)
            .map(|element| element.option_label(host.dom(), option))
            .unwrap_or_default()
    };
    let option_labels = host
        .select_option_elements(node)
        .into_iter()
        .map(|option| option_text(option).into())
        .collect::<Vec<std::sync::Arc<str>>>();
    let maximum = option_labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or_default();
    let selected = host
        .select_selected_option_elements(node)
        .first()
        .copied()
        .map(option_text)
        .unwrap_or_default();
    LayoutFormControlData {
        value: selected.into(),
        option_labels,
        maximum_option_characters: u16::try_from(maximum).unwrap_or(u16::MAX),
        ..form_control_data(element)
    }
}

pub(crate) fn optional_positive_u16_attribute(
    element: &Element,
    name: &str,
    minimum: u16,
    maximum: u16,
) -> Option<u16> {
    element
        .attribute(name)?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .map(|value| value.clamp(u32::from(minimum), u32::from(maximum)) as u16)
}

pub(crate) fn flat_children(host: &DomHost, root: DomHandle, node: DomHandle) -> Vec<DomHandle> {
    let candidates = if let Some(shadow_root) = host.shadow_root_handle(node) {
        host.child_handles(shadow_root).collect::<Vec<_>>()
    } else if host.is_html_element_named(node, "slot") {
        let assigned = host.assigned_nodes_for_slot_with_options(node, false);
        if assigned.is_empty() {
            host.child_handles(node).collect::<Vec<_>>()
        } else {
            assigned
        }
    } else {
        host.child_handles(node).collect::<Vec<_>>()
    };
    candidates
        .into_iter()
        .filter(|child| flat_parent(host, root, *child) == Some(node))
        .collect()
}

pub(crate) fn flat_parent(host: &DomHost, root: DomHandle, node: DomHandle) -> Option<DomHandle> {
    if node == root {
        return None;
    }
    if let Some(slot) = host.assigned_slot_for_node(node) {
        return Some(slot);
    }
    let parent = host.node(node).and_then(Node::parent_node)?;
    if host.is_shadow_root(parent) {
        return host.shadow_root_host(parent);
    }
    if host.is_html_element_named(parent, "slot")
        && !host
            .assigned_nodes_for_slot_with_options(parent, false)
            .is_empty()
    {
        return None;
    }
    if host.shadow_root_handle(parent).is_some() && native_node_is_slotable(host, node) {
        return None;
    }
    Some(parent)
}

fn native_node_is_slotable(host: &DomHost, node: DomHandle) -> bool {
    host.node(node)
        .is_some_and(|node| node.is_element() || node.is_text())
}

//! DOM text and attribute projections shared by resource discovery and rendering.
//! These helpers do not build layout boxes or query geometry.

use moli_layout::LayoutFormControlData;

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

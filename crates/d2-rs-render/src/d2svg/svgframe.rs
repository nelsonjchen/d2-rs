// SPDX-License-Identifier: MPL-2.0
// Port of d2renderers/internal/svgframe: inspect only the opening frame group.
use super::xml::{self, Token};

pub(crate) struct Frame {
    pub master_id: String,
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}

pub(crate) fn read(fragment: &str) -> Option<Frame> {
    // Icons nested inside the frame can have their own viewports and IDs.
    let end = fragment.find('>')?;
    let opening = format!("{}</g>", &fragment[..=end]);
    let tokens = xml::tokens(&opening)?;
    let Token::Start { name, attrs } = tokens.first()? else {
        return None;
    };
    if name.local != "g" {
        return None;
    }
    let attr = |key| {
        attrs
            .iter()
            .find(|a| a.name.local == key)
            .map(|a| a.value.as_str())
    };
    let master_id = attr("class").unwrap_or("").split(' ').next()?.to_owned();
    let mut values = attr("viewBox")?.split_whitespace();
    let left = values.next()?.parse::<i64>().ok()?;
    let top = values.next()?.parse::<i64>().ok()?;
    let width = values.next()?.parse::<i64>().ok()?;
    let height = values.next()?.parse::<i64>().ok()?;
    if width < 0 || height < 0 {
        return None;
    }
    Some(Frame {
        master_id,
        left,
        top,
        right: left.checked_add(width)?,
        bottom: top.checked_add(height)?,
    })
}

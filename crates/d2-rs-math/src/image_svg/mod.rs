// SPDX-License-Identifier: MPL-2.0
//! Shared native imageasset SVG envelope and compression admission.
//! Exporter compatibility helpers retain their original behavior; bounded
//! resolver admission uses caller limits and cancellation explicitly.
#[doc(hidden)]
pub mod go_xml_names;
mod resource;
mod http_content;
pub use http_content::decode_http_content;
#[doc(hidden)]
pub mod svg_asset_codec;
#[doc(hidden)]
pub mod svg_asset_xml;
#[doc(hidden)]
pub mod svg_import_envelope;
pub use resource::{Limits, prepare_resource};

fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(cause) => Err(cause.to_string()),
        None => Ok(()),
    }
}

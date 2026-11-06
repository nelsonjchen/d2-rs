// SPDX-License-Identifier: MPL-2.0
fn decode(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
fn main() {
    let mut rows: Vec<serde_json::Value> = serde_json::from_reader(std::io::stdin()).unwrap();
    for row in &mut rows {
        let error = if let Some(source) = row["source_hex"].as_str() {
            let html = d2_rs_text::markdown::html::render_html_bytes(&decode(source));
            row["html_hex"] =
                serde_json::json!(html.iter().map(|b| format!("{b:02x}")).collect::<String>());
            d2_rs_text::xml::validate_markdown_html_bytes(&html)
                .err()
                .unwrap_or_default()
        } else {
            d2_rs_text::xml::validate_xml_value_bytes(&decode(row["xml_hex"].as_str().unwrap()))
                .err()
                .unwrap_or_default()
        };
        row["xml_error"] = serde_json::json!(error);
    }
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}

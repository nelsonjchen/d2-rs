// SPDX-License-Identifier: MPL-2.0
use d2_rs_tala::ast::Position;
use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let utf16 = std::env::args()
        .skip(1)
        .any(|argument| argument == "--utf16");
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let mut position = Position::default();
    let mut positions = vec![position.debug()];
    for rune in input.chars() {
        position = position.advance(rune, utf16);
        positions.push(position.debug());
    }
    println!("{}", serde_json::to_string(&positions)?);
    Ok(())
}

use super::unicode::{compose, decompose, CharExt};
use unicode_properties::GeneralCategory as G;
fn category(c: char) -> u64 {
    match c.general_category() {
        G::Unassigned => 0,
        G::Control => 1,
        G::Format => 2,
        G::PrivateUse => 3,
        G::Surrogate => 4,
        G::LowercaseLetter => 5,
        G::ModifierLetter => 6,
        G::OtherLetter => 7,
        G::TitlecaseLetter => 8,
        G::UppercaseLetter => 9,
        G::SpacingMark => 10,
        G::EnclosingMark => 11,
        G::NonspacingMark => 12,
        G::DecimalNumber => 13,
        G::LetterNumber => 14,
        G::OtherNumber => 15,
        G::ConnectorPunctuation => 16,
        G::DashPunctuation => 17,
        G::ClosePunctuation => 18,
        G::FinalPunctuation => 19,
        G::InitialPunctuation => 20,
        G::OtherPunctuation => 21,
        G::OpenPunctuation => 22,
        G::CurrencySymbol => 23,
        G::ModifierSymbol => 24,
        G::MathSymbol => 25,
        G::OtherSymbol => 26,
        G::LineSeparator => 27,
        G::ParagraphSeparator => 28,
        G::SpaceSeparator => 29,
    }
}
#[test]
fn all_scalar_properties_match_original_go() {
    let r: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
        "tests/fixtures/go-unicode-v090.json"
    ))
    .unwrap();
    let mut counts = [0usize; 6];
    let mut examples = std::vec::Vec::new();
    let mut scalars = 0;
    let decompositions = r["decomposition"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x[0].as_u64().unwrap() as u32,
                (x[1].as_u64().unwrap() as u32, x[2].as_u64().unwrap() as u32),
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    for row in r["ranges"].as_array().unwrap() {
        for cp in row[0].as_u64().unwrap() as u32..=row[1].as_u64().unwrap() as u32 {
            let c = char::from_u32(cp).unwrap();
            scalars += 1;
            let actual = [
                u64::from(c.script().tag().0),
                category(c),
                super::source_unicode_data::combining_class(c) as u64,
                u64::from(c.mirrored().unwrap_or(c) as u32),
                u64::from(c.modified_combining_class()),
            ];
            let expected = [
                row[2].as_u64().unwrap(),
                row[3].as_u64().unwrap(),
                row[4].as_u64().unwrap(),
                (i64::from(cp) + row[5].as_i64().unwrap()) as u64,
                row[6].as_u64().unwrap(),
            ];
            for i in 0..5 {
                if actual[i] != expected[i] {
                    counts[i] += 1;
                    if examples.len() < 12 {
                        examples.push((cp, i, actual[i], expected[i]));
                    }
                }
            }
            let actual = decompose(c).map(|(a, b)| (a as u32, b as u32));
            if actual != decompositions.get(&cp).copied() {
                counts[5] += 1;
                if examples.len() < 12 {
                    examples.push((cp, 5, 0, 0));
                }
            }
        }
    }
    assert_eq!(scalars, r["scalar_count"].as_u64().unwrap() as usize);
    assert_eq!(
        counts, [0; 6],
        "Source property differences (script/category/CCC/mirror/modifiedCCC/decomposition) {:?}",
        examples
    );
    for row in r["composition"].as_array().unwrap() {
        let a = char::from_u32(row[0].as_u64().unwrap() as u32).unwrap();
        let b = char::from_u32(row[1].as_u64().unwrap() as u32).unwrap();
        assert_eq!(
            compose(a, b).map(|c| c as u32).unwrap_or(0),
            row[2].as_u64().unwrap() as u32,
            "Source stored/Hangul composition pair {:x}+{:x}",
            a as u32,
            b as u32
        );
    }
    for row in r["decomposition"].as_array().unwrap() {
        let a = char::from_u32(row[1].as_u64().unwrap() as u32).unwrap();
        let b = char::from_u32(row[2].as_u64().unwrap() as u32).unwrap();
        assert_eq!(
            compose(a, b).map(|c| c as u32).unwrap_or(0),
            row[3].as_u64().unwrap() as u32,
            "Source reverse composition for U+{:x}",
            row[0].as_u64().unwrap()
        );
    }
}

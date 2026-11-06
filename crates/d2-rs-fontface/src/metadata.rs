// SPDX-License-Identifier: MPL-2.0
//! Face description used by the system fallback resolver, following go-text
//! font/metadata.go and font/opentype/tables/name_src.go.
use ttf_parser::{Face, Tag, name::PlatformId};

pub struct FallbackFaceDescription {
    pub family: String,
    pub italic: bool,
    pub weight: i64,
    pub monospace: bool,
}

const MAC_ROMAN: [u32; 256] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
    98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
    117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 196, 197, 199, 201, 209, 214, 220, 225,
    224, 226, 228, 227, 229, 231, 233, 232, 234, 235, 237, 236, 238, 239, 241, 243, 242, 244, 246,
    245, 250, 249, 251, 252, 8224, 176, 162, 163, 167, 8226, 182, 223, 174, 169, 8482, 180, 168,
    8800, 198, 216, 8734, 177, 8804, 8805, 165, 181, 8706, 8721, 8719, 960, 8747, 170, 186, 937,
    230, 248, 191, 161, 172, 8730, 402, 8776, 8710, 171, 187, 8230, 160, 192, 195, 213, 338, 339,
    8211, 8212, 8220, 8221, 8216, 8217, 247, 9674, 255, 376, 8260, 8364, 8249, 8250, 64257, 64258,
    8225, 183, 8218, 8222, 8240, 194, 202, 193, 203, 200, 205, 206, 207, 204, 211, 212, 63743, 210,
    218, 219, 217, 305, 710, 732, 175, 728, 729, 730, 184, 733, 731, 711,
];

fn name(face: &Face<'_>, id: u16) -> String {
    let (mut unicode, mut apple_roman, mut apple_english, mut win) = (None, None, None, None);
    let mut english = false;
    for record in face
        .names()
        .into_iter()
        .filter(|r| r.name_id == id && !r.name.is_empty())
    {
        match record.platform_id {
            PlatformId::Unicode | PlatformId::Iso => unicode = Some(record),
            PlatformId::Macintosh if record.language_id == 0 => apple_english = Some(record),
            PlatformId::Macintosh if record.encoding_id == 0 => apple_roman = Some(record),
            PlatformId::Windows
                if (win.is_none() || record.language_id & 0x3ff == 9)
                    && matches!(record.encoding_id, 0 | 1 | 10) =>
            {
                english = record.language_id & 0x3ff == 9;
                win = Some(record);
            }
            _ => {}
        }
    }
    let apple = apple_english.or(apple_roman);
    let selected = if win.is_some() && (apple.is_none() || english) {
        win
    } else {
        apple.or(unicode)
    };
    let Some(record) = selected else {
        return String::new();
    };
    if record.platform_id == PlatformId::Unicode
        || (record.platform_id == PlatformId::Windows && matches!(record.encoding_id, 0 | 1 | 10))
    {
        return String::from_utf16_lossy(
            &record
                .name
                .chunks_exact(2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        );
    }
    if record.platform_id == PlatformId::Macintosh && record.encoding_id == 0 {
        return record
            .name
            .iter()
            .map(|&b| char::from_u32(MAC_ROMAN[b as usize]).unwrap())
            .collect();
    }
    String::from_utf8_lossy(record.name).into_owned()
}

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn monospace(face: &Face<'_>) -> bool {
    let table = |tag: &[u8; 4]| face.raw_face().table(Tag::from_bytes(tag));
    if table(b"post")
        .filter(|t| t.len() >= 32)
        .and_then(|t| t.get(12..16))
        .is_some_and(|v| v != [0, 0, 0, 0])
    {
        return true;
    }
    let Some(metrics) = table(b"hmtx") else {
        return false;
    };
    let Some(count) = table(b"hhea").and_then(|h| u16_at(h, 34)) else {
        return false;
    };
    if count == 0 || metrics.len() < count as usize * 4 {
        return false;
    }
    if count == 1 {
        return true;
    }
    let mut first = 0_i32;
    for gid in 1..count as usize {
        let advance = i32::from(u16_at(metrics, gid * 4).unwrap());
        if advance == 0 {
            continue;
        }
        if first == 0 {
            first = advance;
            continue;
        }
        if (advance - first).abs() * 33 > advance.abs().max(first.abs()) {
            return false;
        }
    }
    true
}

/// Metadata as used by go-text's Describe and IsMonospace methods.
pub fn fallback_face_description(face: &Face<'_>) -> FallbackFaceDescription {
    let table = |tag: &[u8; 4]| face.raw_face().table(Tag::from_bytes(tag));
    let os2 = table(b"OS/2").filter(|t| t.len() >= 78);
    let selection = os2.and_then(|t| u16_at(t, 62)).unwrap_or(0);
    let name_from = |ids: &[u16]| {
        ids.iter()
            .map(|&id| name(face, id))
            .find(|s| !s.is_empty())
            .unwrap_or_default()
    };
    let family = if selection & 256 != 0 {
        name_from(&[16, 1])
    } else {
        name_from(&[21, 16, 1])
    };
    let style = if selection & 256 != 0 {
        name_from(&[17, 2])
    } else {
        name_from(&[22, 17, 2])
    };
    let style = style.trim().to_lowercase().replace([' ', '\t'], "");
    let (mut italic, mut weight) = if let Some(os2) = os2 {
        (selection & 513 != 0, i64::from(u16_at(os2, 4).unwrap()))
    } else {
        let mac_style = table(b"head").and_then(|t| u16_at(t, 44)).unwrap_or(0);
        (mac_style & 2 != 0, if mac_style & 1 != 0 { 700 } else { 0 })
    };
    if !italic {
        italic = ["italic", "kursiv", "oblique"]
            .iter()
            .any(|s| style.contains(s));
    }
    if weight == 0 {
        weight = [
            ("thin", 100),
            ("extralight", 200),
            ("ultralight", 200),
            ("light", 300),
            ("demilight", 350),
            ("semilight", 350),
            ("book", 380),
            ("regular", 400),
            ("normal", 400),
            ("medium", 500),
            ("demibold", 600),
            ("demi", 600),
            ("semibold", 600),
            ("extrabold", 800),
            ("superbold", 800),
            ("ultrabold", 800),
            ("bold", 700),
            ("ultrablack", 920),
            ("superblack", 920),
            ("extrablack", 920),
            ("black", 900),
            ("heavy", 900),
        ]
        .iter()
        .find_map(|(s, w)| style.contains(s).then_some(*w))
        .unwrap_or(400);
    }
    FallbackFaceDescription {
        family,
        italic,
        weight: weight.clamp(100, 900),
        monospace: monospace(face),
    }
}

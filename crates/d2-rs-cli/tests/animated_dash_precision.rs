// SPDX-License-Identifier: MPL-2.0
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

#[test]
fn reverse_animated_dash_uses_the_exact_time_zero_phase() {
    // Pinned d2scenebuild keeps the unrounded base dash offset in the scene.
    // The public SVG rounds it to six decimals; using that rounded value for
    // the time-zero raster frame changes this boundary pixel by one level.
    let source = "e-{style.animated:true}\n\
e-nous{style.animated:true}\n\
Friday->Monday{style.animated:true}\n\
Insomnia--p{style.animated:true}\n\
Insomnia--ake{style{}}\n\
Insomnia--m{style{animated:true\n\
stroke-width:8}\n\
label:r}\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_d2-rs"))
        .args([
            "--layout=dagre",
            "--theme=201",
            "--pad=100",
            "--scale=2",
            "--sketch",
            "--center",
            "--stdout-format=png",
            "-",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut reader = png::Decoder::new(Cursor::new(output.stdout))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((frame.width, frame.height), (3812, 1820));
    assert_eq!(frame.color_type, png::ColorType::Rgb);
    let offset = (691 * frame.width as usize + 3015) * 3;
    assert_eq!(&pixels[offset..offset + 3], &[188, 190, 196]);
}

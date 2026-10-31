// SPDX-License-Identifier: MPL-2.0
//! Input-only replay of the original paged CLI link-rebase calls.
use super::*;

#[test]
#[ignore = "executed explicitly by cli-mode-scoreboard.py --suite paged-export"]
fn replay() {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_PAGED_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<_> = requests
        .iter()
        .map(|input| {
            assert_eq!(input["operation"], "rebase");
            let original: Vec<d2_rs_scene::LinkRegion> =
                serde_json::from_value(input["links"].clone()).unwrap();
            let source_root = input["source_root"].as_str().unwrap();
            let selected = source_root.strip_prefix("root.").unwrap();
            let pages = input["pages"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            // The production packaging adapter includes Go's rebase operation.
            // Box and tooltip are input metadata for this observation. Every
            // original link must produce one metadata entry, with no filtering.
            let result = links(original.clone(), selected, &pages, ExportFormat::Pptx);
            let (rebased, error) = match result {
                Ok(metadata) => {
                    assert_eq!(metadata.len(), original.len());
                    let values: Vec<_> = original
                        .iter()
                        .cloned()
                        .zip(metadata)
                        .map(|(mut region, link)| {
                            if link.board_target {
                                region.target = link.region.href;
                                region.url.clear();
                            } else {
                                region.target.clear();
                                region.url = link.region.href;
                            }
                            region
                        })
                        .collect();
                    (Some(values), String::new())
                }
                Err(error) => (None, error),
            };
            serde_json::json!({"links":rebased,"input_links":original,"error":error})
        })
        .collect();
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::env::var_os("D2RS_PAGED_RESPONSE").unwrap())
        .unwrap();
    serde_json::to_writer(&mut output, &results).unwrap();
}

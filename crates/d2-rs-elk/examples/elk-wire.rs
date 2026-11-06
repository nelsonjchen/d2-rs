//! Development-only adapter for the translated ELK processor chain.
use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let mut graph: d2_rs_elk::ElkGraph = serde_json::from_str(&input)?;
    // Compare raw wire graphs with elk-go's defaults. The public D2 adapter
    // writes its own spacing choices into the graph and hydrates them there.
    let options = d2_rs_elk::ElkOptions {
        node_spacing: 20.0,
        edge_node_spacing: 10.0,
        ..d2_rs_elk::ElkOptions::default()
    };
    #[cfg(feature = "diagnostic-traces")]
    {
        let events = d2_rs_elk::layout_elk_graph_with_layered_pipeline_trace(&mut graph, options)?;
        eprintln!("{}", serde_json::to_string(&events)?);
    }
    #[cfg(not(feature = "diagnostic-traces"))]
    d2_rs_elk::layout_elk_graph_with_layered_pipeline(&mut graph, options)?;
    serde_json::to_writer(std::io::stdout().lock(), &graph)?;
    std::io::stdout().write_all(b"\n")?;
    Ok(())
}

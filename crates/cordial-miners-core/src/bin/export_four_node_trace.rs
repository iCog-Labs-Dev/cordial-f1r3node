use std::path::PathBuf;

use cordial_miners_core::simulation::trace::write_four_node_trace_js;

fn main() {
    let output = PathBuf::from("docs/cordial-miners/generated/four-node-trace.js");
    write_four_node_trace_js(&output)
        .unwrap_or_else(|error| panic!("failed to write trace to {}: {error}", output.display()));
    println!("wrote {}", output.display());
}

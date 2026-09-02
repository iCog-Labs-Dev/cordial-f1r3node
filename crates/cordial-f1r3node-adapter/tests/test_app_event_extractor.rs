use std::collections::BTreeMap;

use cordial_f1r3node_adapter::app_event_extractor::{AppEventExtractionInput, extract_app_events};
use cordial_f1r3node_adapter::ordered_output::OrderedFinalizedOutput;

#[test]
fn empty_ordered_output_produces_no_app_events() {
    let input = AppEventExtractionInput {
        ordered_output: OrderedFinalizedOutput::default(),
        deploys_by_block_hash: BTreeMap::new(),
    };

    let events = extract_app_events(input);

    assert!(events.is_empty());
}

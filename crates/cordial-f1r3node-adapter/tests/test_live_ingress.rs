use cordial_f1r3node_adapter::live_ingress::{LiveIngress, LiveIngressPhase};

#[test]
fn new_live_ingress_starts_in_defined_phase() {
    let ingress = LiveIngress::new(());
    assert_eq!(ingress.phase(), LiveIngressPhase::Defined);
}

#[test]
fn live_ingress_phase_can_progress_without_changing_adapter() {
    let mut ingress = LiveIngress::new(String::from("adapter"));

    ingress.mark_traced();
    assert_eq!(ingress.phase(), LiveIngressPhase::Traced);
    assert_eq!(ingress.adapter(), "adapter");

    ingress.mark_connected();
    assert_eq!(ingress.phase(), LiveIngressPhase::Connected);
    assert_eq!(ingress.into_inner(), "adapter");
}

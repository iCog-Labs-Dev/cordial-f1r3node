use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(f1r3node_has_listen_for_data_at_name)");
    println!("cargo::rustc-check-cfg=cfg(f1r3node_has_deploy_finalization_status)");

    let proto = PathBuf::from("../../../f1r3node/models/src/main/protobuf/DeployServiceV1.proto");
    let Ok(contents) = fs::read_to_string(&proto) else {
        return;
    };

    if contents.contains("listenForDataAtName") {
        println!("cargo:rustc-cfg=f1r3node_has_listen_for_data_at_name");
    }

    if contents.contains("deployFinalizationStatus") {
        println!("cargo:rustc-cfg=f1r3node_has_deploy_finalization_status");
    }
}

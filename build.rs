use std::env;
use std::path::PathBuf;

fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("failed to find protoc");
    std::env::set_var("PROTOC", protoc_path);

    println!("cargo:rerun-if-changed=proto/users.proto");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .file_descriptor_set_path(out_dir.join("users_descriptor.bin"))
        .compile(&["proto/users.proto"], &["proto"])
        .expect("failed to compile protobuf definitions");
}

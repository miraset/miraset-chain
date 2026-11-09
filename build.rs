//! Compile `proto/signer.proto` with the pure-Rust `protox` compiler, so building needs no
//! system `protoc` (locally or in the Docker image).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/signer.proto");
    let descriptors = protox::compile(["proto/signer.proto"], ["proto"])?;
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_fds(descriptors)?;
    Ok(())
}

use std::io::Result;

fn main() -> Result<()> {
    #[allow(unused_mut)]
    let mut builder = tonic_build::configure();

    builder
        .build_server(true)
        .build_client(true)
        .emit_rerun_if_changed(false)
        .bytes(["."])
        .extern_path(".tvix.castore.v1", "::tvix_castore::proto")
        .compile_protos(
            &[
                "tvix/build/protos/build.proto",
                "tvix/build/protos/rpc_build.proto",
            ],
            // If we are in running `cargo build` manually, using `../..` works fine,
            // but in case we run inside a nix build, we need to instead point PROTO_ROOT
            // to a custom tree containing that structure.
            &[match std::env::var_os("PROTO_ROOT") {
                Some(proto_root) => proto_root.to_str().unwrap().to_owned(),
                None => "../..".to_string(),
            }],
        )?;

    Ok(())
}

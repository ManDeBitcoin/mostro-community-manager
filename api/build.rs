fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Explicit compiler path avoids changing process environment in edition 2024.
    let mut config = tonic_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    tonic_build::configure()
        .build_server(false)
        .compile_protos_with_config(
            config,
            &["../config/upstream/admin.proto"],
            &["../config/upstream"],
        )?;
    println!("cargo:rerun-if-changed=../config/upstream/admin.proto");
    Ok(())
}

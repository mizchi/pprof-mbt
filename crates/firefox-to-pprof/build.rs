fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/profile.proto");
    prost_build::compile_protos(&["proto/profile.proto"], &["proto/"])?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	tonic_prost_build::configure().build_server(true).out_dir("./src").build_client(true).compile_protos(&["./asset.proto"], &["./"])?;
	Ok(())
}

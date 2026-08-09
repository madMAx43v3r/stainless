fn main() {
    stainless_build::Builder::new("src/client.stl")
        .add_source("src/server.stl")
        .output_name("http.stainless.rs")
        .compile()
        .unwrap_or_else(|error| panic!("{error}"));
}

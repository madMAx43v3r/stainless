fn main() {
    stainless_build::Builder::new("src/http.stl")
        .output_name("http.stainless.rs")
        .compile()
        .unwrap_or_else(|error| panic!("{error}"));
}

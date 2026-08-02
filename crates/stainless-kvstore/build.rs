fn main() {
    stainless_build::Builder::new("src/kvstore.stl")
        .output_name("kvstore.stainless.rs")
        .export("kvstore::self_test", "stainless_kvstore_self_test")
        .compile()
        .unwrap_or_else(|error| panic!("{error}"));
}

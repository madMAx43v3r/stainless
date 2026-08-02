fn main() {
    stainless_build::Builder::new("src/kvstore.stl")
        .output_name("kvstore.stainless.rs")
        .export("kvstore::open_table", "stainless_kvstore_open_raw")
        .export("kvstore::insert_bytes", "stainless_kvstore_insert_raw")
        .export("kvstore::find_bytes", "stainless_kvstore_find_raw")
        .export(
            "kvstore::find_range_bytes",
            "stainless_kvstore_find_range_raw",
        )
        .export(
            "kvstore::find_range_first_bytes",
            "stainless_kvstore_find_range_first_raw",
        )
        .export(
            "kvstore::find_range_last_bytes",
            "stainless_kvstore_find_range_last_raw",
        )
        .export("kvstore::commit_table", "stainless_kvstore_commit_raw")
        .export("kvstore::revert_table", "stainless_kvstore_revert_raw")
        .export("kvstore::table_version", "stainless_kvstore_version_raw")
        .export("kvstore::self_test", "stainless_kvstore_self_test")
        .compile()
        .unwrap_or_else(|error| panic!("{error}"));
}

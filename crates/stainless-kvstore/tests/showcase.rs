use std::time::{SystemTime, UNIX_EPOCH};

use stainless_kvstore::{Codec, Table};

fn temporary_log(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock follows Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "stainless-kvstore-{label}-{}-{unique}.log",
        std::process::id()
    ))
}

fn index_log(path: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.index", path.to_string_lossy()))
}

fn remove_logs(path: &std::path::Path) {
    std::fs::remove_file(path).expect("data WAL should be removable");
    std::fs::remove_file(index_log(path)).expect("index WAL should be removable");
}

#[test]
fn stainless_store_commits_reverts_recovers_and_reads_concurrently() {
    let path = temporary_log("raw");

    stainless_kvstore::self_test(&path.to_string_lossy())
        .unwrap_or_else(|error| panic!("showcase failed: {error}"));
}

#[test]
fn typed_store_persists_compound_keys_and_generic_values() {
    let path = temporary_log("typed");
    let table = Table::<(u32, String), String>::open(&path)
        .unwrap_or_else(|error| panic!("open failed: {error}"));
    let key = (7, "alice".to_owned());

    table
        .insert(key.clone(), "first".to_owned())
        .unwrap_or_else(|error| panic!("insert failed: {error}"));
    assert!(table.commit(1).expect("first commit should succeed"));
    table
        .insert(key.clone(), "second".to_owned())
        .unwrap_or_else(|error| panic!("second insert failed: {error}"));
    assert!(table.commit(2).expect("second commit should succeed"));
    assert_eq!(
        table.find(&key).expect("find should succeed"),
        Some("second".to_owned())
    );
    assert!(table.revert(1).expect("revert should succeed"));
    drop(table);

    let recovered = Table::<(u32, String), String>::open(&path)
        .unwrap_or_else(|error| panic!("recovery failed: {error}"));
    assert_eq!(recovered.current_version(), 1);
    assert_eq!(
        recovered.find(&key).expect("recovered find should succeed"),
        Some("first".to_owned())
    );
    remove_logs(&path);
}

#[test]
fn range_find_returns_latest_values_and_a_reverse_bounded_tail() {
    let path = temporary_log("range-find");
    let table = Table::<u32, u32>::open(&path).expect("open should succeed");

    for key in 1..=5 {
        table.insert(key, key * 10).expect("initial insert");
    }
    assert!(table.commit(1).expect("first commit"));
    table.insert(3, 300).expect("replacement insert");
    assert!(table.commit(2).expect("second commit"));

    assert_eq!(
        table.find_range(&2, &4).expect("range lookup"),
        [(2, 20), (3, 300), (4, 40)]
    );
    assert_eq!(
        table
            .find_range_first(&1, &5, 3)
            .expect("forward prefix lookup"),
        [(1, 10), (2, 20), (3, 300)]
    );
    assert_eq!(
        table
            .find_range_last(&1, &5, 3)
            .expect("reverse tail lookup"),
        [(5, 50), (4, 40), (3, 300)]
    );
    assert!(
        table
            .find_range_first(&1, &5, 0)
            .expect("empty forward prefix lookup")
            .is_empty()
    );
    assert!(
        table
            .find_range_last(&1, &5, 0)
            .expect("empty reverse tail lookup")
            .is_empty()
    );
    assert!(
        table
            .find_range(&5, &1)
            .expect("reversed bounds are empty")
            .is_empty()
    );

    drop(table);
    remove_logs(&path);
}

#[test]
fn compound_key_codec_preserves_tuple_order() {
    let mut keys = [
        (2_u32, "alpha".to_owned()),
        (1_u32, "zeta".to_owned()),
        (1_u32, "alpha".to_owned()),
    ];
    keys.sort();
    let mut encoded = keys
        .iter()
        .map(|key| key.encode().expect("compound key should encode"))
        .collect::<Vec<_>>();
    let already_sorted = encoded.clone();
    encoded.sort();
    assert_eq!(encoded, already_sorted);
}

#[test]
fn wal_integer_fields_use_fixed_width_big_endian_encoding() {
    let path = temporary_log("big-endian");
    let table =
        Table::<u32, u32>::open(&path).unwrap_or_else(|error| panic!("open failed: {error}"));
    table
        .insert(0x0102_0304, 0x0506_0708)
        .unwrap_or_else(|error| panic!("insert failed: {error}"));
    assert!(table.commit(1).expect("commit should succeed"));
    drop(table);

    let bytes = std::fs::read(&path).expect("data WAL should be readable");
    assert_eq!(bytes.len(), 33);
    assert_eq!(&bytes[0..4], &33_u32.to_be_bytes());
    assert_eq!(bytes[4], 0);
    assert_eq!(&bytes[5..9], &0_u32.to_be_bytes());
    assert_eq!(&bytes[9..17], &4_u64.to_be_bytes());
    assert_eq!(&bytes[17..21], &4_u32.to_be_bytes());
    assert_eq!(&bytes[21..25], &0x0102_0304_u32.to_be_bytes());
    assert_eq!(&bytes[25..29], &0x0506_0708_u32.to_be_bytes());

    let index = std::fs::read(index_log(&path)).expect("index WAL should be readable");
    assert_eq!(index.len(), 37 + 21);
    assert_eq!(&index[0..4], &37_u32.to_be_bytes());
    assert_eq!(index[4], 0);
    assert_eq!(&index[5..9], &0_u32.to_be_bytes());
    assert_eq!(&index[9..17], &25_u64.to_be_bytes());
    assert_eq!(&index[17..21], &4_u32.to_be_bytes());
    assert_eq!(&index[21..29], &4_u64.to_be_bytes());
    assert_eq!(&index[29..33], &0x0102_0304_u32.to_be_bytes());
    assert_eq!(&index[33..37], &0x0222_002c_u32.to_be_bytes());

    assert_eq!(&index[37..41], &21_u32.to_be_bytes());
    assert_eq!(index[41], 1);
    assert_eq!(&index[42..46], &1_u32.to_be_bytes());
    assert_eq!(&index[46..54], &33_u64.to_be_bytes());
    assert_eq!(&index[54..58], &0x0044_0024_u32.to_be_bytes());

    remove_logs(&path);
}

#[test]
fn revert_truncates_only_the_recent_index_wal_branch() {
    let path = temporary_log("index-revert");
    let table = Table::<u32, u32>::open(&path).expect("open should succeed");

    table.insert(1, 10).expect("version zero insert");
    assert!(table.commit(1).expect("commit one"));
    let retained_index_len = std::fs::metadata(index_log(&path))
        .expect("index WAL metadata")
        .len();

    table.insert(2, 20).expect("abandoned version one insert");
    assert!(table.commit(2).expect("commit two"));
    table.insert(3, 30).expect("abandoned version two insert");
    assert!(table.commit(3).expect("commit three"));
    assert!(table.revert(1).expect("revert should succeed"));

    assert_eq!(
        std::fs::metadata(index_log(&path))
            .expect("truncated index WAL metadata")
            .len(),
        retained_index_len
    );
    assert_eq!(table.find(&1).expect("retained lookup"), Some(10));
    assert_eq!(table.find(&2).expect("discarded lookup"), None);
    assert_eq!(table.find(&3).expect("discarded lookup"), None);

    table.insert(3, 31).expect("replacement branch insert");
    assert!(table.commit(2).expect("replacement branch commit"));
    drop(table);

    let recovered = Table::<u32, u32>::open(&path).expect("recovery should succeed");
    assert_eq!(
        recovered.find(&1).expect("recovered retained lookup"),
        Some(10)
    );
    assert_eq!(
        recovered.find(&2).expect("old branch must stay absent"),
        None
    );
    assert_eq!(recovered.find(&3).expect("replacement lookup"), Some(31));
    drop(recovered);
    remove_logs(&path);
}

#[test]
fn recovery_discards_only_the_uncommitted_index_suffix() {
    let path = temporary_log("uncommitted-index");
    let table = Table::<u32, u32>::open(&path).expect("open should succeed");
    table.insert(1, 10).expect("committed insert");
    assert!(table.commit(1).expect("commit should succeed"));
    let committed_data_len = std::fs::metadata(&path).expect("data metadata").len();
    let committed_index_len = std::fs::metadata(index_log(&path))
        .expect("index metadata")
        .len();
    table.insert(2, 20).expect("uncommitted insert");
    drop(table);

    let recovered = Table::<u32, u32>::open(&path).expect("recovery should succeed");
    assert_eq!(recovered.find(&1).expect("committed lookup"), Some(10));
    assert_eq!(recovered.find(&2).expect("uncommitted lookup"), None);
    assert_eq!(
        std::fs::metadata(&path)
            .expect("truncated data metadata")
            .len(),
        committed_data_len
    );
    assert_eq!(
        std::fs::metadata(index_log(&path))
            .expect("truncated index metadata")
            .len(),
        committed_index_len
    );
    drop(recovered);
    remove_logs(&path);
}

#[test]
fn revert_uses_the_least_successor_for_an_unindexed_version() {
    let path = temporary_log("index-version-index-successor");
    let table = Table::<u32, u32>::open(&path).expect("open should succeed");

    table.insert(1, 10).expect("version zero insert");
    assert!(table.commit(5).expect("commit five"));
    table.insert(2, 20).expect("version five insert");
    assert!(table.commit(10).expect("commit ten"));

    assert!(table.revert(7).expect("revert seven"));
    assert_eq!(table.current_version(), 7);
    assert_eq!(table.find(&1).expect("version zero lookup"), Some(10));
    assert_eq!(table.find(&2).expect("version five lookup"), Some(20));
    drop(table);

    let recovered = Table::<u32, u32>::open(&path).expect("reopen should succeed");
    assert_eq!(recovered.current_version(), 7);
    assert_eq!(recovered.find(&1).expect("recovered first key"), Some(10));
    assert_eq!(recovered.find(&2).expect("recovered second key"), Some(20));
    drop(recovered);
    remove_logs(&path);
}

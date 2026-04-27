use diff_lib::storage::Storage;
use tempfile::TempDir;

#[test]
fn record_and_list_roundtrip() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("state.db");
    let storage = Storage::open_at(&db).unwrap();

    assert!(storage.list_for_host("local").unwrap().is_empty());

    storage.record_open("local", "/home/u/projects/a").unwrap();
    storage.record_open("host1", "/srv/projects/b").unwrap();
    storage.record_open("local", "/home/u/projects/c").unwrap();

    let local = storage.list_for_host("local").unwrap();
    assert_eq!(local.len(), 2);
    // Most recent first
    assert_eq!(local[0].path, "/home/u/projects/c");

    let host = storage.list_for_host("host1").unwrap();
    assert_eq!(host.len(), 1);
    assert_eq!(host[0].path, "/srv/projects/b");
}

#[test]
fn record_open_updates_timestamp_on_conflict() {
    let dir = TempDir::new().unwrap();
    let storage = Storage::open_at(&dir.path().join("s.db")).unwrap();
    storage.record_open("local", "/x").unwrap();
    let t1 = storage.list_for_host("local").unwrap()[0].last_opened_at;
    std::thread::sleep(std::time::Duration::from_secs(1));
    storage.record_open("local", "/x").unwrap();
    let t2 = storage.list_for_host("local").unwrap()[0].last_opened_at;
    assert!(t2 > t1);
}

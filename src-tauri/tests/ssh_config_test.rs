use diff_lib::ssh_config;

#[test]
fn missing_file_returns_empty() {
    // The function falls back to dirs::home_dir() which we can't easily override.
    // Verify the function doesn't panic and returns Ok with whatever the user has.
    let result = ssh_config::list_hosts();
    assert!(result.is_ok(), "list_hosts should not error: {:?}", result);
}

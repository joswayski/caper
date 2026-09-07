use std::process::Command;

#[test]
fn migration_command_requires_its_own_url_without_runtime_fallback() {
    for value in [None, Some(""), Some("   ")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_caper-api"));
        command
            .env_clear()
            .arg("--migrate")
            .env(
                "DATABASE_URL",
                "postgres://unused:secret@127.0.0.1:1/caperchat",
            )
            .env("MEDIA_ENABLED", "true");
        if let Some(value) = value {
            command.env("MIGRATION_DATABASE_URL", value);
        }
        let result = command.output().unwrap();
        assert!(!result.status.success());
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(output.contains("MIGRATION_DATABASE_URL is required"));
        assert!(!output.contains("secret"));
    }
}

#[test]
fn migration_command_rejects_pooling_and_invalid_urls_before_connecting() {
    for (url, expected) in [
        (
            "postgres://unused:secret@127.0.0.1:6432/caperchat",
            "direct connection",
        ),
        (
            "mysql://unused:secret@127.0.0.1/caperchat",
            "MIGRATION_DATABASE_URL must be a PostgreSQL URL",
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_caper-api"))
            .env_clear()
            .arg("--migrate")
            .env("MIGRATION_DATABASE_URL", url)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(output.contains(expected));
        assert!(!output.contains("secret"));
    }
}

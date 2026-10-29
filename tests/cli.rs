use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after Unix epoch")
            .as_nanos();
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("envui-cli-{}-{nonce}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("temporary test directory should be created");
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_envui"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("envui process should start")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn list_prints_entries_from_dotenv_file() {
    let dir = TestDir::new();
    fs::write(
        dir.0.join(".env"),
        "APP_MODE=testing\nGREETING=\"hello world\"\n",
    )
    .expect("dotenv fixture should be written");

    let output = dir.run(&["--list", "--dotenv"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("SOURCE\tKEY\tVALUE"));
    assert!(stdout.contains(".env\tAPP_MODE\ttesting"));
    assert!(stdout.contains(".env\tGREETING\thello world"));
}

#[test]
fn help_shows_usage_without_opening_the_tui() {
    let dir = TestDir::new();

    let output = dir.run(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Usage: envui [--list] [--dotenv]"));
    assert!(stdout.contains("User and System variables"));
    assert!(stdout.contains("--dotenv to edit .env"));
}

#[test]
fn unsupported_arguments_return_an_error() {
    let dir = TestDir::new();

    let output = dir.run(&["--unknown"]);

    assert!(!output.status.success());
}

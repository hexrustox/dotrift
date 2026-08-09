use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use dotrift::state::StateDatabase;
use tempfile::TempDir;

static STATE_HOME_LOCK: Mutex<()> = Mutex::new(());

pub struct TestEnv {
    _guard: MutexGuard<'static, ()>,
    root: TempDir,
}

impl TestEnv {
    pub fn new() -> Self {
        let guard = STATE_HOME_LOCK.lock().expect("state home lock poisoned");
        let root = TempDir::new().expect("cannot create temp dir");
        unsafe {
            std::env::set_var("XDG_STATE_HOME", root.path().join("state"));
        }
        Self {
            _guard: guard,
            root,
        }
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn database(&self) -> StateDatabase {
        StateDatabase::open().expect("cannot open state database")
    }
}

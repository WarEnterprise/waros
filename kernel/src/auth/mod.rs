pub mod login;
pub mod passwords;
pub mod permissions;
pub mod session;
pub mod users;

use core::sync::atomic::{AtomicBool, Ordering};

use spin::{Lazy, Mutex};

pub use permissions::FilePermissions;
pub use users::{AuthError, UserAccount, UserDB, UserRole};

pub static USER_DB: Lazy<Mutex<UserDB>> = Lazy::new(|| Mutex::new(UserDB::with_default_root()));
static FIRST_BOOT_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy)]
pub struct AuthInitReport {
    pub first_boot: bool,
    pub users: usize,
}

pub fn init() -> Result<AuthInitReport, AuthError> {
    let (db, first_boot) = match UserDB::load_from_fs() {
        Some(db) => (db, false),
        None => {
            let db = UserDB::with_default_root();
            db.save_to_fs();
            (db, true)
        }
    };
    let users = db.list_users().len();
    *USER_DB.lock() = db;
    FIRST_BOOT_PENDING.store(first_boot, Ordering::Relaxed);
    Ok(AuthInitReport { first_boot, users })
}

#[must_use]
pub fn username_for_uid(uid: u16) -> alloc::string::String {
    USER_DB
        .lock()
        .find_by_uid(uid)
        .map(|user| user.username.clone())
        .unwrap_or_else(|| alloc::format!("uid{}", uid))
}

#[must_use]
pub fn first_boot_pending() -> bool {
    FIRST_BOOT_PENDING.load(Ordering::Relaxed)
}

pub fn clear_first_boot_pending() {
    FIRST_BOOT_PENDING.store(false, Ordering::Relaxed);
}

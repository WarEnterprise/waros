use alloc::string::{String, ToString};

use spin::Mutex;

use super::{permissions, UserAccount, UserRole};

pub static CURRENT_SESSION: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Debug, Clone)]
pub struct Session {
    pub user: UserAccount,
    pub cwd: String,
    pub started_at: u64,
}

impl Session {
    #[must_use]
    pub fn new(user: UserAccount) -> Self {
        let cwd = user.home_dir.clone();
        Self {
            user,
            cwd,
            started_at: crate::arch::x86_64::interrupts::tick_count(),
        }
    }

    #[must_use]
    pub fn uid(&self) -> u16 {
        self.user.uid
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.user.username
    }

    #[must_use]
    pub fn role(&self) -> UserRole {
        self.user.role
    }

    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.user.role == UserRole::Admin
    }

    #[must_use]
    pub fn home(&self) -> &str {
        &self.user.home_dir
    }

    #[must_use]
    pub fn prompt_path(&self) -> String {
        if self.cwd == self.user.home_dir {
            String::from("~")
        } else if self.cwd.starts_with(&self.user.home_dir) {
            alloc::format!("~{}", &self.cwd[self.user.home_dir.len()..])
        } else {
            self.cwd.clone()
        }
    }
}

pub fn start(user: UserAccount) {
    *CURRENT_SESSION.lock() = Some(Session::new(user));
}

pub fn logout() {
    *CURRENT_SESSION.lock() = None;
}

#[must_use]
pub fn current_user() -> Option<UserAccount> {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(|session| session.user.clone())
}

#[must_use]
pub fn is_logged_in() -> bool {
    CURRENT_SESSION.lock().is_some()
}

#[must_use]
pub fn is_admin() -> bool {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .is_some_and(Session::is_admin)
}

#[must_use]
pub fn current_uid() -> u16 {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(Session::uid)
        .unwrap_or(0)
}

#[must_use]
pub fn current_username() -> String {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(|session| session.username().to_string())
        .unwrap_or_else(|| String::from("unknown"))
}

#[must_use]
pub fn current_role() -> UserRole {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(Session::role)
        .unwrap_or(UserRole::Admin)
}

#[must_use]
pub fn current_home() -> String {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(|session| session.home().to_string())
        .unwrap_or_else(|| String::from("/"))
}

#[must_use]
pub fn current_prompt_path() -> String {
    CURRENT_SESSION
        .lock()
        .as_ref()
        .map(Session::prompt_path)
        .unwrap_or_else(|| String::from("/"))
}

#[must_use]
pub fn resolve_path(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return current_home();
    }

    let session = CURRENT_SESSION.lock();
    let Some(session) = session.as_ref() else {
        return trimmed.to_string();
    };

    if trimmed == "~" {
        return session.home().to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return alloc::format!("{}/{}", session.home(), rest);
    }
    if trimmed.starts_with('/') {
        return trimmed.to_string();
    }
    alloc::format!("{}/{}", session.cwd, trimmed)
}

#[must_use]
pub fn can_access_path(path: &str, write: bool) -> bool {
    let Some(user) = current_user() else {
        return true;
    };
    if user.role == UserRole::Admin {
        return true;
    }

    let path = path.trim();
    let Some((owner_uid, _)) = permissions::protected_path_owner(path) else {
        return true;
    };

    let _ = write;
    owner_uid == user.uid
}

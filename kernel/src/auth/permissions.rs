use alloc::string::String;

use super::users::UserRole;
use crate::auth::USER_DB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilePermissions {
    pub owner_uid: u16,
    pub owner_read: bool,
    pub owner_write: bool,
    pub others_read: bool,
    pub others_write: bool,
}

impl FilePermissions {
    #[must_use]
    pub fn default_for(uid: u16) -> Self {
        Self {
            owner_uid: uid,
            owner_read: true,
            owner_write: true,
            others_read: false,
            others_write: false,
        }
    }

    #[must_use]
    pub fn private(uid: u16) -> Self {
        Self::default_for(uid)
    }

    #[must_use]
    pub fn system() -> Self {
        Self {
            owner_uid: 0,
            owner_read: true,
            owner_write: true,
            others_read: true,
            others_write: false,
        }
    }

    #[must_use]
    pub fn shared(uid: u16) -> Self {
        Self {
            owner_uid: uid,
            owner_read: true,
            owner_write: true,
            others_read: true,
            others_write: false,
        }
    }

    #[must_use]
    pub fn can_read(&self, uid: u16, role: UserRole) -> bool {
        if role == UserRole::Admin {
            return true;
        }
        if uid == self.owner_uid {
            return self.owner_read;
        }
        self.others_read
    }

    #[must_use]
    pub fn can_write(&self, uid: u16, role: UserRole) -> bool {
        if role == UserRole::Admin {
            return true;
        }
        if uid == self.owner_uid {
            return self.owner_write;
        }
        self.others_write
    }

    #[must_use]
    pub fn mode_string(&self) -> String {
        alloc::format!(
            "{}{}{}{}",
            if self.owner_read { "r" } else { "-" },
            if self.owner_write { "w" } else { "-" },
            if self.others_read { "r" } else { "-" },
            if self.others_write { "w" } else { "-" },
        )
    }

    pub fn apply_mode_string(&mut self, mode: &str) -> bool {
        let bytes = mode.as_bytes();
        if bytes.len() != 4 {
            return false;
        }
        self.owner_read = matches!(bytes[0], b'r' | b'R');
        self.owner_write = matches!(bytes[1], b'w' | b'W');
        self.others_read = matches!(bytes[2], b'r' | b'R');
        self.others_write = matches!(bytes[3], b'w' | b'W');
        matches!(bytes[0], b'r' | b'R' | b'-')
            && matches!(bytes[1], b'w' | b'W' | b'-')
            && matches!(bytes[2], b'r' | b'R' | b'-')
            && matches!(bytes[3], b'w' | b'W' | b'-')
    }
}

#[must_use]
pub fn protected_path_owner(path: &str) -> Option<(u16, String)> {
    if path == "/root" || path.starts_with("/root/") {
        return Some((0, String::from("root")));
    }

    let remainder = path.strip_prefix("/home/")?;
    let username = remainder.split('/').next()?;
    USER_DB
        .lock()
        .find_by_name(username)
        .map(|user| (user.uid, user.username.clone()))
}


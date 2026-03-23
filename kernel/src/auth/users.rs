use alloc::string::String;
use alloc::vec::Vec;

use crate::auth::passwords::{constant_time_eq, generate_salt, hash_password};

pub const MAX_USERS: usize = 32;
const USERS_DB_PATH: &str = "/etc/users.db";

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    Admin = 0,
    User = 1,
    Guest = 2,
}

impl UserRole {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
            Self::Guest => "guest",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UserAccount {
    pub uid: u16,
    pub username: String,
    pub password_hash: [u8; 32],
    pub salt: [u8; 16],
    pub role: UserRole,
    pub home_dir: String,
    pub active: bool,
    pub created_at: u64,
    pub last_login: u64,
}

#[derive(Debug, Clone)]
pub struct UserDB {
    users: Vec<UserAccount>,
    next_uid: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    InvalidUsername,
    InvalidPassword,
    UserExists,
    UserNotFound,
    WrongPassword,
    AccountDisabled,
    TooManyUsers,
    CannotDeleteRoot,
    PermissionDenied,
    NotLoggedIn,
    SerializationError,
}

impl UserDB {
    #[must_use]
    pub fn with_default_root() -> Self {
        let mut db = Self {
            users: Vec::new(),
            next_uid: 0,
        };
        let _ = db.create_user("root", "waros", UserRole::Admin);
        db
    }

    pub fn create_user(
        &mut self,
        username: &str,
        password: &str,
        role: UserRole,
    ) -> Result<u16, AuthError> {
        if !is_valid_username(username) {
            return Err(AuthError::InvalidUsername);
        }
        if password.len() < 4 {
            return Err(AuthError::InvalidPassword);
        }
        if self.find_by_name(username).is_some() {
            return Err(AuthError::UserExists);
        }
        if self.users.len() >= MAX_USERS {
            return Err(AuthError::TooManyUsers);
        }

        let uid = self.next_uid;
        self.next_uid = self.next_uid.saturating_add(1);
        let salt = generate_salt(uid);
        let password_hash = hash_password(password, &salt);
        let now = crate::arch::x86_64::interrupts::tick_count();
        let home_dir = if uid == 0 {
            String::from("/root")
        } else {
            alloc::format!("/home/{}", username)
        };

        self.users.push(UserAccount {
            uid,
            username: String::from(username),
            password_hash,
            salt,
            role,
            home_dir,
            active: true,
            created_at: now,
            last_login: 0,
        });
        self.users.sort_by(|left, right| left.uid.cmp(&right.uid));
        Ok(uid)
    }

    pub fn authenticate(
        &self,
        username: &str,
        password: &str,
    ) -> Result<UserAccount, AuthError> {
        let user = self.find_by_name(username).ok_or(AuthError::UserNotFound)?;
        if !user.active {
            return Err(AuthError::AccountDisabled);
        }

        let actual = hash_password(password, &user.salt);
        if !constant_time_eq(&actual, &user.password_hash) {
            return Err(AuthError::WrongPassword);
        }
        Ok(user.clone())
    }

    #[must_use]
    pub fn find_by_name(&self, username: &str) -> Option<&UserAccount> {
        self.users.iter().find(|user| user.username == username)
    }

    pub fn find_mut_by_name(&mut self, username: &str) -> Option<&mut UserAccount> {
        self.users.iter_mut().find(|user| user.username == username)
    }

    #[must_use]
    pub fn find_by_uid(&self, uid: u16) -> Option<&UserAccount> {
        self.users.iter().find(|user| user.uid == uid)
    }

    pub fn find_mut_by_uid(&mut self, uid: u16) -> Option<&mut UserAccount> {
        self.users.iter_mut().find(|user| user.uid == uid)
    }

    #[must_use]
    pub fn list_users(&self) -> &[UserAccount] {
        &self.users
    }

    pub fn change_password(&mut self, uid: u16, new_password: &str) -> Result<(), AuthError> {
        if new_password.len() < 4 {
            return Err(AuthError::InvalidPassword);
        }

        let user = self.find_mut_by_uid(uid).ok_or(AuthError::UserNotFound)?;
        let salt = generate_salt(uid);
        user.salt = salt;
        user.password_hash = hash_password(new_password, &salt);
        Ok(())
    }

    pub fn delete_user(&mut self, uid: u16) -> Result<(), AuthError> {
        if uid == 0 {
            return Err(AuthError::CannotDeleteRoot);
        }
        let index = self
            .users
            .iter()
            .position(|user| user.uid == uid)
            .ok_or(AuthError::UserNotFound)?;
        self.users.remove(index);
        Ok(())
    }

    pub fn record_login(&mut self, uid: u16) -> Result<u64, AuthError> {
        let now = crate::arch::x86_64::interrupts::tick_count();
        let user = self.find_mut_by_uid(uid).ok_or(AuthError::UserNotFound)?;
        let previous = user.last_login;
        user.last_login = now;
        Ok(previous)
    }

    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&(self.users.len() as u16).to_le_bytes());
        data.extend_from_slice(&self.next_uid.to_le_bytes());

        for user in &self.users {
            data.extend_from_slice(&user.uid.to_le_bytes());
            data.push(user.username.len() as u8);
            data.extend_from_slice(user.username.as_bytes());
            data.extend_from_slice(&user.password_hash);
            data.extend_from_slice(&user.salt);
            data.push(user.role as u8);
            data.push(user.home_dir.len() as u8);
            data.extend_from_slice(user.home_dir.as_bytes());
            data.push(user.active as u8);
            data.extend_from_slice(&user.created_at.to_le_bytes());
            data.extend_from_slice(&user.last_login.to_le_bytes());
        }

        data
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, AuthError> {
        let mut cursor = 0usize;
        let user_count = read_u16(data, &mut cursor)? as usize;
        let next_uid = read_u16(data, &mut cursor)?;
        let mut users = Vec::with_capacity(user_count);

        for _ in 0..user_count {
            let uid = read_u16(data, &mut cursor)?;
            let username_len = read_u8(data, &mut cursor)? as usize;
            let username = read_string(data, &mut cursor, username_len)?;

            let mut password_hash = [0u8; 32];
            password_hash.copy_from_slice(read_bytes(data, &mut cursor, 32)?);
            let mut salt = [0u8; 16];
            salt.copy_from_slice(read_bytes(data, &mut cursor, 16)?);

            let role = match read_u8(data, &mut cursor)? {
                0 => UserRole::Admin,
                1 => UserRole::User,
                2 => UserRole::Guest,
                _ => return Err(AuthError::SerializationError),
            };

            let home_len = read_u8(data, &mut cursor)? as usize;
            let home_dir = read_string(data, &mut cursor, home_len)?;
            let active = read_u8(data, &mut cursor)? != 0;
            let created_at = read_u64(data, &mut cursor)?;
            let last_login = read_u64(data, &mut cursor)?;

            users.push(UserAccount {
                uid,
                username,
                password_hash,
                salt,
                role,
                home_dir,
                active,
                created_at,
                last_login,
            });
        }

        Ok(Self { users, next_uid })
    }

    pub fn save_to_fs(&self) {
        let data = self.serialize();
        let _ = crate::fs::FILESYSTEM
            .lock()
            .write_system(USERS_DB_PATH, &data, false);
    }

    #[must_use]
    pub fn load_from_fs() -> Option<Self> {
        let filesystem = crate::fs::FILESYSTEM.lock();
        let data = filesystem.read(USERS_DB_PATH).ok()?;
        Self::deserialize(data).ok()
    }
}

impl core::fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUsername => formatter.write_str("invalid username"),
            Self::InvalidPassword => formatter.write_str("invalid password"),
            Self::UserExists => formatter.write_str("user already exists"),
            Self::UserNotFound => formatter.write_str("user not found"),
            Self::WrongPassword => formatter.write_str("incorrect password"),
            Self::AccountDisabled => formatter.write_str("account disabled"),
            Self::TooManyUsers => formatter.write_str("user database is full"),
            Self::CannotDeleteRoot => formatter.write_str("cannot delete root"),
            Self::PermissionDenied => formatter.write_str("permission denied"),
            Self::NotLoggedIn => formatter.write_str("not logged in"),
            Self::SerializationError => formatter.write_str("user database is corrupted"),
        }
    }
}

fn is_valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= 32
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn read_u8(data: &[u8], cursor: &mut usize) -> Result<u8, AuthError> {
    let byte = *data.get(*cursor).ok_or(AuthError::SerializationError)?;
    *cursor += 1;
    Ok(byte)
}

fn read_u16(data: &[u8], cursor: &mut usize) -> Result<u16, AuthError> {
    let bytes = read_bytes(data, cursor, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u64(data: &[u8], cursor: &mut usize) -> Result<u64, AuthError> {
    let bytes = read_bytes(data, cursor, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_string(data: &[u8], cursor: &mut usize, len: usize) -> Result<String, AuthError> {
    let bytes = read_bytes(data, cursor, len)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| AuthError::SerializationError)
}

fn read_bytes<'a>(
    data: &'a [u8],
    cursor: &mut usize,
    len: usize,
) -> Result<&'a [u8], AuthError> {
    let end = cursor.saturating_add(len);
    let bytes = data
        .get(*cursor..end)
        .ok_or(AuthError::SerializationError)?;
    *cursor = end;
    Ok(bytes)
}


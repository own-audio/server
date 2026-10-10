// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a password has to be (ASVS 2.1, security hardening plan §5.1):
//! twelve characters or more, nothing else. No composition rules — a digit
//! and a capital letter make `Password1` no stronger, and the rule people
//! actually follow is "a few words". The limit on the long side only keeps
//! the hash cheap. Existing passwords are not re-checked; the rule applies
//! when one is set.
use crate::auth::error::AuthError;

pub const MIN_CHARS: usize = 12;
pub const MAX_CHARS: usize = 256;

pub fn check(password: &str) -> Result<(), AuthError> {
    let chars = password.chars().count();
    if chars < MIN_CHARS {
        return Err(AuthError::BadRequest(format!(
            "password must be at least {MIN_CHARS} characters; a few words are fine"
        )));
    }
    if chars > MAX_CHARS {
        return Err(AuthError::BadRequest(format!("password must be at most {MAX_CHARS} characters")));
    }
    if password.trim().is_empty() {
        return Err(AuthError::BadRequest("password must not be blank".into()));
    }
    Ok(())
}

/// How long an account waits after `failures` wrong passwords in a row:
/// nothing for the first few (people mistype), then a doubling delay from
/// 30 seconds up to 16 minutes. Per account, not per address, so a slow
/// guess spread over many addresses still runs into it.
pub fn lock_after(failures: i32) -> Option<std::time::Duration> {
    if failures < LOCK_AT {
        return None;
    }
    let step = (failures - LOCK_AT).min(5) as u32;
    Some(std::time::Duration::from_secs(30 * 2u64.pow(step)))
}

/// The failure count at which the lock starts and the owner is told.
pub const LOCK_AT: i32 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_characters_of_anything() {
        assert!(check("correct horse battery").is_ok());
        assert!(check("ěščřžýáíéůúň").is_ok(), "counted in characters, not bytes");
        assert!(check("elevenchars").is_err());
        assert!(check("            ").is_err(), "blank");
        assert!(check(&"x".repeat(257)).is_err());
    }

    #[test]
    fn the_lock_grows_and_stops_growing() {
        assert_eq!(lock_after(4), None);
        assert_eq!(lock_after(5).unwrap().as_secs(), 30);
        assert_eq!(lock_after(6).unwrap().as_secs(), 60);
        assert_eq!(lock_after(10).unwrap().as_secs(), 960);
        assert_eq!(lock_after(50).unwrap().as_secs(), 960);
    }
}

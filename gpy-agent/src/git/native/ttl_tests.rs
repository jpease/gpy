//! The untracked-cache recheck has its own lifetime, independent of the
//! language cache TTL.

use super::{UNTRACKED_CACHE_CHECKED, UNTRACKED_CACHE_RECHECK};
use crate::cache::ttl_map::LanguageTtlGuard;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

#[test]
#[serial_test::serial(global_ttl)]
fn untracked_cache_check_ignores_the_language_ttl() {
    let _ttl = LanguageTtlGuard::set(1);
    let repo = PathBuf::from("untracked-cache-ttl-independence");
    let now = SystemTime::now();

    UNTRACKED_CACHE_CHECKED.insert_at(repo.clone(), (), now - Duration::from_hours(2));
    assert!(
        UNTRACKED_CACHE_CHECKED.get(&repo).is_some(),
        "a 2h-old check must survive a 1h language TTL"
    );

    UNTRACKED_CACHE_CHECKED.insert_at(
        repo.clone(),
        (),
        now - UNTRACKED_CACHE_RECHECK - Duration::from_secs(5),
    );
    assert!(
        UNTRACKED_CACHE_CHECKED.get(&repo).is_none(),
        "a check older than the fixed recheck period expires"
    );
}

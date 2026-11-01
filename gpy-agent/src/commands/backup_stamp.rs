//! The `YYYYMMDD_HHMMSS` suffix of backup file names.

/// `YYYYMMDD_HHMMSS` (UTC) for `epoch_secs`, the suffix the installers write
/// and `gpy uninstall` restores. Civil-from-days after Howard Hinnant.
#[allow(
    clippy::arithmetic_side_effects,
    reason = "days is at most u64::MAX / 86_400, so no step overflows, and each subtraction is non-negative by the civil-from-days algorithm"
)]
pub(super) fn backup_stamp(epoch_secs: u64) -> String {
    let days = epoch_secs / 86_400;
    let secs_of_day = epoch_secs % 86_400;

    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);

    format!(
        "{year:04}{month:02}{day:02}_{:02}{:02}{:02}",
        secs_of_day / 3_600,
        secs_of_day % 3_600 / 60,
        secs_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::backup_stamp;

    // Each expected value is `date -u -r N +%Y%m%d_%H%M%S`.

    #[test]
    fn epoch() {
        assert_eq!(backup_stamp(0), "19700101_000000");
    }

    #[test]
    fn leap_day() {
        assert_eq!(backup_stamp(951_782_400), "20000229_000000");
    }

    #[test]
    fn time_of_day() {
        assert_eq!(backup_stamp(1_760_485_533), "20251014_234533");
    }

    #[test]
    fn year_end() {
        assert_eq!(backup_stamp(4_102_444_799), "20991231_235959");
    }
}

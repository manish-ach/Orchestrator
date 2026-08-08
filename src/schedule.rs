// Five-field cron, just enough to answer "does this minute match?".
//
// Hand-rolled rather than pulled in as a dependency, because the scheduler
// below ticks once a minute and asks exactly one question. Computing the *next*
// occurrence — the hard part a cron library exists for — is never needed: the
// ticker already knows what minute it is.
//
//   ┌── minute (0-59)
//   │ ┌── hour (0-23)
//   │ │ ┌── day of month (1-31)
//   │ │ │ ┌── month (1-12)
//   │ │ │ │ ┌── day of week (0-6, Sunday = 0; 7 also accepted)
//   * * * * *
//
// Supported per field: `*`, `n`, `a-b`, `a,b,c`, `*/n`, `a-b/n`.

use chrono::{DateTime, Datelike, Local, Timelike};

#[derive(Debug, Clone, PartialEq)]
struct Field {
    /// which values in the field's range are allowed
    allowed: Vec<u32>,
    /// true when the field was a bare `*` — needed for the day-of-month /
    /// day-of-week rule below
    wildcard: bool,
}

impl Field {
    fn parse(spec: &str, min: u32, max: u32) -> Result<Field, String> {
        if spec == "*" {
            return Ok(Field { allowed: (min..=max).collect(), wildcard: true });
        }
        let mut allowed = Vec::new();
        for part in spec.split(',') {
            let (range, step) = match part.split_once('/') {
                Some((r, s)) => (r, s.parse::<u32>().map_err(|_| format!("bad step in '{part}'"))?),
                None => (part, 1),
            };
            if step == 0 {
                return Err(format!("step cannot be zero in '{part}'"));
            }
            let (lo, hi) = if range == "*" {
                (min, max)
            } else if let Some((a, b)) = range.split_once('-') {
                (
                    a.parse::<u32>().map_err(|_| format!("bad range start in '{part}'"))?,
                    b.parse::<u32>().map_err(|_| format!("bad range end in '{part}'"))?,
                )
            } else {
                let v = range.parse::<u32>().map_err(|_| format!("bad value '{part}'"))?;
                (v, v)
            };
            if lo > hi {
                return Err(format!("range is backwards in '{part}'"));
            }
            if lo < min || hi > max {
                return Err(format!("'{part}' is outside {min}-{max}"));
            }
            allowed.extend((lo..=hi).step_by(step as usize));
        }
        if allowed.is_empty() {
            return Err(format!("'{spec}' matches nothing"));
        }
        allowed.sort_unstable();
        allowed.dedup();
        Ok(Field { allowed, wildcard: false })
    }

    fn has(&self, v: u32) -> bool {
        self.allowed.binary_search(&v).is_ok()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

/// Parse a five-field expression. Rejects anything it does not fully
/// understand rather than silently ignoring it — a schedule that quietly means
/// something other than what it says is worse than one that fails loudly.
pub fn parse(expr: &str) -> Result<Cron, String> {
    let f: Vec<&str> = expr.split_whitespace().collect();
    if f.len() != 5 {
        return Err(format!("expected 5 fields, got {} in '{expr}'", f.len()));
    }
    Ok(Cron {
        minute: Field::parse(f[0], 0, 59)?,
        hour: Field::parse(f[1], 0, 23)?,
        dom: Field::parse(f[2], 1, 31)?,
        month: Field::parse(f[3], 1, 12)?,
        // 7 means Sunday too, as every crontab accepts
        dow: Field::parse(&f[4].replace('7', "0"), 0, 6)?,
    })
}

impl Cron {
    /// Does `t` fall on this schedule, to the minute?
    pub fn matches(&self, t: DateTime<Local>) -> bool {
        if !self.minute.has(t.minute()) || !self.hour.has(t.hour()) || !self.month.has(t.month()) {
            return false;
        }
        let dom_ok = self.dom.has(t.day());
        let dow_ok = self.dow.has(t.weekday().num_days_from_sunday());
        // Standard cron: when BOTH day fields are restricted they are ORed, not
        // ANDed — `0 0 13 * 5` is "the 13th, and every Friday", not "Friday the
        // 13th". When only one is restricted, the wildcard one is ignored.
        match (self.dom.wildcard, self.dow.wildcard) {
            (true, true) => true,
            (false, true) => dom_ok,
            (true, false) => dow_ok,
            (false, false) => dom_ok || dow_ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().expect("valid local time")
    }

    #[test]
    fn every_minute() {
        let c = parse("* * * * *").unwrap();
        assert!(c.matches(at(2026, 8, 7, 3, 14)));
        assert!(c.matches(at(2026, 1, 1, 0, 0)));
    }

    #[test]
    fn a_fixed_time_matches_only_that_minute() {
        let c = parse("30 2 * * *").unwrap();
        assert!(c.matches(at(2026, 8, 7, 2, 30)));
        assert!(!c.matches(at(2026, 8, 7, 2, 31)));
        assert!(!c.matches(at(2026, 8, 7, 3, 30)));
    }

    #[test]
    fn steps_lists_and_ranges() {
        let c = parse("*/15 9-17 * * 1-5").unwrap();
        assert!(c.matches(at(2026, 8, 7, 9, 0)));   // Friday 09:00
        assert!(c.matches(at(2026, 8, 7, 17, 45)));
        assert!(!c.matches(at(2026, 8, 7, 9, 7)));  // not a quarter hour
        assert!(!c.matches(at(2026, 8, 7, 8, 0)));  // before the window
        assert!(!c.matches(at(2026, 8, 8, 9, 0)));  // Saturday
    }

    #[test]
    fn a_list_of_hours() {
        let c = parse("0 0,6,12,18 * * *").unwrap();
        for h in [0, 6, 12, 18] {
            assert!(c.matches(at(2026, 8, 7, h, 0)), "hour {h} should match");
        }
        assert!(!c.matches(at(2026, 8, 7, 7, 0)));
    }

    /// The classic cron gotcha. Getting this wrong silently runs a job on the
    /// wrong days, which is exactly the kind of bug nobody notices for months.
    #[test]
    fn restricted_day_fields_are_ored_not_anded() {
        let c = parse("0 0 13 * 5").unwrap();
        assert!(c.matches(at(2026, 3, 13, 0, 0)), "the 13th, a Friday");
        assert!(c.matches(at(2026, 8, 13, 0, 0)), "the 13th, a Thursday — still matches");
        assert!(c.matches(at(2026, 8, 7, 0, 0)), "a Friday that is not the 13th — still matches");
        assert!(!c.matches(at(2026, 8, 6, 0, 0)), "Thursday the 6th matches neither");
    }

    #[test]
    fn one_restricted_day_field_ignores_the_wildcard_one() {
        let weekdays = parse("0 3 * * 1-5").unwrap();
        assert!(weekdays.matches(at(2026, 8, 7, 3, 0)), "Friday");
        assert!(!weekdays.matches(at(2026, 8, 9, 3, 0)), "Sunday");

        let first = parse("0 3 1 * *").unwrap();
        assert!(first.matches(at(2026, 8, 1, 3, 0)));
        assert!(!first.matches(at(2026, 8, 2, 3, 0)));
    }

    #[test]
    fn sunday_is_both_zero_and_seven() {
        assert_eq!(parse("0 0 * * 0").unwrap(), parse("0 0 * * 7").unwrap());
        assert!(parse("0 0 * * 7").unwrap().matches(at(2026, 8, 9, 0, 0)));
    }

    #[test]
    fn nonsense_is_rejected_rather_than_guessed() {
        for bad in ["", "* * * *", "* * * * * *", "60 * * * *", "* 24 * * *", "0 0 0 * *", "*/0 * * * *", "5-1 * * * *", "abc * * * *"] {
            assert!(parse(bad).is_err(), "'{bad}' should not parse");
        }
    }
}

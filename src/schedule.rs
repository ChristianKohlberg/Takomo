//! The cadence of a schedule, and the clock that turns it into occurrence
//! instants. See spec/schedule-format.md.
//!
//! Pure by design: no store, no HTTP, no `ApiError`. Like [`crate::workflow`],
//! parsing and validation return plain `String`/`Vec<String>` messages and the
//! API layer wraps them in a 422 — which keeps every calendar edge case in this
//! file testable with `#[cfg(test)]` units instead of a live server.
//!
//! Two invariants the rest of the feature rests on:
//!
//! - **A slot is computed from the cadence alone.** Nothing here reads a ticket,
//!   a previous occurrence, or the database. That is what makes occurrences
//!   independent of each other: the only thing two occurrences of one schedule
//!   share is the link back to it.
//! - **An occurrence's deadline is simply the next occurrence.** So
//!   `expires_at` needs no field of its own — it is
//!   `next_slot_after(slot, anchor)`, stamped when the ticket is created.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// Weekday tokens accepted in `on`, Monday first.
pub const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Recurrence units accepted in `every`.
pub const UNITS: [&str; 3] = ["day", "week", "month"];

/// Upper bound on `interval`. 52 covers "every 52 weeks" (a year) and "every 52
/// months" (over four years); past that a schedule is really a reminder for a
/// date, which is a ticket, not a cadence.
pub const MAX_INTERVAL: u32 = 52;

/// How far forward [`Cadence::next_slot_after`] will look for a matching day
/// before giving up. Six years, so even `interval: 52` on a weekday cadence has
/// room; a `None` past this means the cadence matches nothing, not that the
/// search was too short.
const MAX_DAY_SCAN: u32 = 366 * 6;

/// Same, counted in months for the monthly unit.
const MAX_MONTH_SCAN: u32 = 12 * 60;

/// How far past a requested wall-clock time to look for one that exists, when a
/// DST jump has deleted it. Real gaps are 30–60 minutes; 240 leaves margin
/// without letting a pathological zone spin.
const MAX_GAP_MINUTES: i64 = 240;

fn default_interval() -> u32 {
    1
}

/// The recurrence rule, as authored.
///
/// `deny_unknown_fields` is the point of the struct, not decoration: `ony:
/// [mon]` has to be a hard error, because the alternative is a schedule that
/// silently drops the weekday filter and fires every single day. Same reasoning
/// as [`crate::workflow`], where a mistyped `require:` would silently delete an
/// approval gate.
///
/// Fields are plain `String`s rather than enums so a wrong value gets a message
/// naming the legal ones — serde's own "unknown variant" text is written for a
/// Rust programmer, and the reader here is usually an LLM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cadence {
    /// `day` | `week` | `month`.
    pub every: String,
    /// Repeat every N units. Counted from the schedule's anchor, so "every 2
    /// weeks" means the same weeks forever rather than drifting.
    #[serde(default = "default_interval")]
    pub interval: u32,
    /// Weekday tokens; `week` only, and required there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on: Vec<String>,
    /// Day of month, 1–31, clamped to the month's length; `month` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<u32>,
    /// Local wall-clock time, `HH:MM`, 24-hour.
    pub at: String,
    /// IANA zone name. `None` means UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
}

impl Cadence {
    /// Parse a cadence from a request body value.
    ///
    /// Handles the input we most expect to be wrong — a cron expression — by
    /// answering with its translation rather than growing a second dialect. The
    /// caller wraps the returned message in `validation.schedule.cadence`.
    pub fn parse(value: &serde_json::Value) -> Result<Cadence, String> {
        if let Some(s) = value.as_str() {
            return Err(cron_hint(s));
        }
        if !value.is_object() {
            return Err(format!(
                "A cadence must be an object like {}, not {}. Fields: every (day|week|month), \
                 interval (optional, default 1), on (week only), day (month only), at (HH:MM), \
                 tz (optional IANA name, default UTC).",
                r#"{"every":"week","on":["mon"],"at":"09:00","tz":"Europe/Berlin"}"#,
                kind_of(value),
            ));
        }
        let cadence: Cadence = serde_json::from_value(value.clone()).map_err(|e| {
            format!(
                "Could not read the cadence: {e}. Fields: every (day|week|month), interval \
                 (optional, default 1), on (week only), day (month only), at (HH:MM), tz \
                 (optional IANA name, default UTC). Unknown fields are refused on purpose — a \
                 typo like 'ony' would otherwise drop the weekday filter and fire every day."
            )
        })?;
        let problems = cadence.validate();
        if !problems.is_empty() {
            return Err(problems.join(" "));
        }
        Ok(cadence)
    }

    /// Every reason this cadence is unusable, so one response can report them
    /// all rather than making the caller fix them one round-trip at a time.
    pub fn validate(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();

        if !UNITS.contains(&self.every.as_str()) {
            out.push(format!(
                "every must be one of {}, got '{}'.",
                UNITS.join(", "),
                self.every
            ));
        }
        if !(1..=MAX_INTERVAL).contains(&self.interval) {
            out.push(format!(
                "interval must be between 1 and {MAX_INTERVAL}, got {}.",
                self.interval
            ));
        }
        if parse_hhmm(&self.at).is_none() {
            out.push(format!(
                "at must be a 24-hour local time 'HH:MM' (e.g. \"09:00\"), got '{}'.",
                self.at
            ));
        }
        if let Some(name) = &self.tz {
            if name.parse::<Tz>().is_err() {
                out.push(format!(
                    "tz must be an IANA zone name (e.g. \"Europe/Berlin\", \"UTC\"), got '{name}'."
                ));
            }
        }

        // Unit-specific fields. Setting one that does not apply is an error and
        // not an ignored extra: `every: day, on: [mon]` almost certainly means
        // the author wanted a weekly cadence, and firing daily would be the
        // opposite of their intent.
        match self.every.as_str() {
            "week" => {
                if self.on.is_empty() {
                    out.push(format!(
                        "a week cadence needs at least one weekday in on ({}).",
                        WEEKDAYS.join(", ")
                    ));
                }
                for token in &self.on {
                    if !WEEKDAYS.contains(&token.as_str()) {
                        out.push(format!(
                            "on contains '{token}', which is not a weekday ({}).",
                            WEEKDAYS.join(", ")
                        ));
                    }
                }
                if self.day.is_some() {
                    out.push(
                        "day applies to a month cadence only; a week cadence selects days with on."
                            .to_string(),
                    );
                }
            }
            "month" => {
                if let Some(d) = self.day {
                    if !(1..=31).contains(&d) {
                        out.push(format!(
                            "day must be between 1 and 31, got {d}. A day past the end of a short \
                             month is clamped to that month's last day."
                        ));
                    }
                }
                if !self.on.is_empty() {
                    out.push(
                        "on applies to a week cadence only; a month cadence selects a date with day."
                            .to_string(),
                    );
                }
            }
            "day" => {
                if !self.on.is_empty() {
                    out.push(
                        "on applies to a week cadence only. Did you mean every: week?".to_string(),
                    );
                }
                if self.day.is_some() {
                    out.push(
                        "day applies to a month cadence only. Did you mean every: month?"
                            .to_string(),
                    );
                }
            }
            _ => {} // already reported above
        }
        out
    }

    /// The zone slots are computed in. UTC when `tz` is unset.
    pub fn timezone(&self) -> Tz {
        self.tz
            .as_deref()
            .and_then(|n| n.parse::<Tz>().ok())
            .unwrap_or(chrono_tz::UTC)
    }

    /// The first occurrence strictly after `after`.
    ///
    /// `anchor` is what `interval` counts from — the schedule's `starts_at`, or
    /// its creation time. Anchoring rather than using calendar parity is what
    /// keeps "every 2 weeks" landing on the same weeks a year later instead of
    /// flipping whenever the year has 53 of them.
    ///
    /// `None` means the cadence matches no instant within the search horizon,
    /// which for a validated cadence cannot happen; it is not "no slot today".
    pub fn next_slot_after(
        &self,
        after: DateTime<Utc>,
        anchor: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        let tz = self.timezone();
        let (hh, mm) = parse_hhmm(&self.at)?;
        let anchor_date = anchor.with_timezone(&tz).date_naive();
        let from_date = after.with_timezone(&tz).date_naive();

        if self.every == "month" {
            let (mut year, mut month) = (from_date.year(), from_date.month());
            for _ in 0..MAX_MONTH_SCAN {
                if self.month_matches(year, month, anchor_date) {
                    let day = clamp_day_of_month(year, month, self.day.unwrap_or(1));
                    if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                        if let Some(instant) = resolve_local(&tz, date, hh, mm) {
                            if instant > after {
                                return Some(instant);
                            }
                        }
                    }
                }
                (year, month) = next_month(year, month);
            }
            return None;
        }

        let mut date = from_date;
        for _ in 0..MAX_DAY_SCAN {
            if self.day_matches(date, anchor_date) {
                if let Some(instant) = resolve_local(&tz, date, hh, mm) {
                    if instant > after {
                        return Some(instant);
                    }
                }
            }
            date = date.succ_opt()?;
        }
        None
    }

    /// When an occurrence at `slot` stops counting as live work: the moment its
    /// successor comes due. There is no `expires_at` field on a cadence because
    /// there does not need to be one.
    pub fn expires_at(&self, slot: DateTime<Utc>, anchor: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.next_slot_after(slot, anchor)
    }

    /// Substitute the four occurrence placeholders in a ticket-template string.
    ///
    /// Deliberately four and not a template language: enough to make each
    /// occurrence nameable on the board, nothing that needs its own docs.
    /// `{date}`, `{week}` and `{month}` are local to the cadence's zone because
    /// they are labels for a human; `{slot}` is the UTC instant, because it is
    /// the occurrence's identity.
    pub fn render(&self, template: &str, slot: DateTime<Utc>) -> String {
        let local = slot.with_timezone(&self.timezone());
        let iso = local.iso_week();
        template
            .replace("{date}", &local.format("%Y-%m-%d").to_string())
            .replace("{week}", &format!("{}-W{:02}", iso.year(), iso.week()))
            .replace("{month}", &local.format("%Y-%m").to_string())
            .replace(
                "{slot}",
                &slot.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            )
    }

    fn day_matches(&self, date: NaiveDate, anchor: NaiveDate) -> bool {
        match self.every.as_str() {
            "day" => {
                let elapsed = (date - anchor).num_days();
                elapsed.rem_euclid(self.interval as i64) == 0
            }
            "week" => {
                let token = WEEKDAYS[date.weekday().num_days_from_monday() as usize];
                if !self.on.iter().any(|t| t == token) {
                    return false;
                }
                let weeks = (week_start(date) - week_start(anchor)).num_days() / 7;
                weeks.rem_euclid(self.interval as i64) == 0
            }
            _ => false,
        }
    }

    fn month_matches(&self, year: i32, month: u32, anchor: NaiveDate) -> bool {
        let elapsed = (year - anchor.year()) as i64 * 12 + (month as i64 - anchor.month() as i64);
        elapsed.rem_euclid(self.interval as i64) == 0
    }
}

/// Turn a local wall-clock time into an instant, resolving both DST anomalies.
///
/// - **Spring forward** deletes the requested time. Clamp to the first minute
///   that does exist, rather than dropping the occurrence: a missed occurrence
///   has to be a decision somebody made, never an artifact of the calendar.
/// - **Fall back** makes it happen twice. Take the earlier, so which one fires
///   is deterministic rather than whichever the library happened to return.
fn resolve_local(tz: &Tz, date: NaiveDate, hh: u32, mm: u32) -> Option<DateTime<Utc>> {
    let wanted = date.and_hms_opt(hh, mm, 0)?;
    for minutes in 0..=MAX_GAP_MINUTES {
        let candidate = wanted + Duration::minutes(minutes);
        match tz.from_local_datetime(&candidate) {
            LocalResult::Single(dt) => return Some(dt.with_timezone(&Utc)),
            LocalResult::Ambiguous(earlier, _later) => return Some(earlier.with_timezone(&Utc)),
            LocalResult::None => continue,
        }
    }
    None
}

/// `HH:MM`, 24-hour, both parts required. Returns `None` for anything else —
/// including "9:00", which is rejected so stored cadences stay comparable as
/// strings.
fn parse_hhmm(raw: &str) -> Option<(u32, u32)> {
    let (h, m) = raw.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (hh, mm) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    if hh > 23 || mm > 59 {
        return None;
    }
    Some((hh, mm))
}

/// The Monday of `date`'s week, so week arithmetic does not depend on where a
/// year boundary falls.
fn week_start(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = next_month(year, month);
    let first = NaiveDate::from_ymd_opt(year, month, 1);
    let next_first = NaiveDate::from_ymd_opt(ny, nm, 1);
    match (first, next_first) {
        (Some(a), Some(b)) => (b - a).num_days() as u32,
        _ => 28,
    }
}

/// "The 31st" plainly means month-end to whoever wrote it, so February gets the
/// 28th (or the 29th in a leap year) rather than being skipped.
fn clamp_day_of_month(year: i32, month: u32, day: u32) -> u32 {
    day.min(days_in_month(year, month)).max(1)
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// A cron expression arrived where a cadence belongs. Rather than maintain a
/// second dialect, translate it in the error — the caller needed that
/// translation anyway, and it costs one message instead of a parser.
fn cron_hint(raw: &str) -> String {
    let head = "takomo cadences are declarative objects, not cron strings.";
    let fields: Vec<&str> = raw.split_whitespace().collect();
    let tail = "Cron carries no timezone, so set tz explicitly (IANA name, e.g. \
                \"Europe/Berlin\") or it defaults to UTC.";

    if fields.len() != 5 {
        return format!(
            "{head} Use e.g. {}. {tail}",
            r#"{"every":"week","on":["mon"],"at":"09:00","tz":"Europe/Berlin"}"#
        );
    }
    let (min, hour, dom, _mon, dow) = (fields[0], fields[1], fields[2], fields[3], fields[4]);
    let simple = |f: &str| f.parse::<u32>().ok();

    let at = match (simple(min), simple(hour)) {
        (Some(m), Some(h)) if m <= 59 && h <= 23 => format!("{h:02}:{m:02}"),
        _ => {
            return format!(
                "{head} '{raw}' varies the minute or hour, which a cadence cannot express — it \
                 fires once per slot at one wall-clock time. Pick a single time: {}. {tail}",
                r#"{"every":"day","at":"09:00"}"#
            );
        }
    };

    let equivalent = if dow != "*" {
        let days: Vec<String> = dow
            .split(',')
            .filter_map(|d| cron_weekday(d).map(|w| format!("\"{w}\"")))
            .collect();
        if days.is_empty() {
            None
        } else {
            Some(format!(
                r#"{{"every":"week","on":[{}],"at":"{at}","tz":"UTC"}}"#,
                days.join(",")
            ))
        }
    } else if let Some(d) = simple(dom) {
        Some(format!(
            r#"{{"every":"month","day":{d},"at":"{at}","tz":"UTC"}}"#
        ))
    } else if dom == "*" {
        Some(format!(r#"{{"every":"day","at":"{at}","tz":"UTC"}}"#))
    } else {
        None
    };

    match equivalent {
        Some(eq) => format!("{head} The equivalent of '{raw}' is {eq}. {tail}"),
        None => format!(
            "{head} '{raw}' has no direct equivalent. Use e.g. {}. {tail}",
            r#"{"every":"week","on":["mon"],"at":"09:00","tz":"Europe/Berlin"}"#
        ),
    }
}

/// Cron day-of-week to a cadence weekday token. Accepts both the numeric form
/// (0 and 7 are Sunday, as every cron does) and the three-letter names.
fn cron_weekday(field: &str) -> Option<&'static str> {
    let lower = field.to_ascii_lowercase();
    if let Some(w) = WEEKDAYS.iter().find(|w| **w == lower) {
        return Some(w);
    }
    match lower.parse::<u32>() {
        Ok(0) | Ok(7) => Some("sun"),
        Ok(n) if (1..=6).contains(&n) => Some(WEEKDAYS[(n - 1) as usize]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn cadence(json: serde_json::Value) -> Cadence {
        Cadence::parse(&json).expect("cadence should be valid")
    }

    // ---- the ordinary path -------------------------------------------------

    #[test]
    fn weekly_monday_finds_the_next_monday_in_local_time() {
        let c = cadence(serde_json::json!({
            "every": "week", "on": ["mon"], "at": "09:00", "tz": "Europe/Berlin"
        }));
        let anchor = utc("2026-07-01T00:00:00Z");
        // Wednesday 29 July 2026 → Monday 3 August, 09:00 CEST = 07:00 UTC.
        let slot = c
            .next_slot_after(utc("2026-07-29T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-08-03T07:00:00Z"));
    }

    #[test]
    fn no_tz_means_utc() {
        let c = cadence(serde_json::json!({ "every": "day", "at": "06:30" }));
        let anchor = utc("2026-01-01T00:00:00Z");
        let slot = c
            .next_slot_after(utc("2026-07-29T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-07-30T06:30:00Z"));
    }

    #[test]
    fn the_boundary_is_strict_so_a_slot_never_returns_itself() {
        let c = cadence(serde_json::json!({ "every": "day", "at": "06:30" }));
        let anchor = utc("2026-01-01T00:00:00Z");
        let slot = utc("2026-07-30T06:30:00Z");
        assert_eq!(
            c.next_slot_after(slot, anchor).unwrap(),
            utc("2026-07-31T06:30:00Z"),
            "asking from exactly a slot must advance, or materialize_due would loop on it"
        );
    }

    #[test]
    fn several_weekdays_pick_the_nearest() {
        let c = cadence(serde_json::json!({
            "every": "week", "on": ["mon", "thu"], "at": "09:00"
        }));
        let anchor = utc("2026-07-01T00:00:00Z");
        // Tuesday 28 July 2026 → Thursday the 30th, not the following Monday.
        let slot = c
            .next_slot_after(utc("2026-07-28T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-07-30T09:00:00Z"));
    }

    // ---- expiry is just the next slot --------------------------------------

    #[test]
    fn expiry_is_the_following_occurrence() {
        let c = cadence(serde_json::json!({
            "every": "week", "on": ["mon"], "at": "09:00", "tz": "Europe/Berlin"
        }));
        let anchor = utc("2026-07-01T00:00:00Z");
        let slot = utc("2026-08-03T07:00:00Z");
        assert_eq!(
            c.expires_at(slot, anchor).unwrap(),
            utc("2026-08-10T07:00:00Z"),
            "a weekly occurrence is stale exactly when the next one is due"
        );
    }

    // ---- daylight saving ---------------------------------------------------

    #[test]
    fn spring_forward_clamps_into_the_gap_rather_than_skipping_the_day() {
        // Europe/Berlin springs forward on Sunday 29 March 2026: 02:00 → 03:00,
        // so 02:30 local does not exist that day.
        let c = cadence(serde_json::json!({
            "every": "day", "at": "02:30", "tz": "Europe/Berlin"
        }));
        // Pin the premise. If a tzdata update ever moves this transition, the
        // assertions below would still pass while testing nothing at all, so
        // prove the gap exists before relying on it.
        let tz = "Europe/Berlin".parse::<Tz>().unwrap();
        let gap = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(
            matches!(tz.from_local_datetime(&gap), LocalResult::None),
            "2026-03-29 02:30 must not exist in Europe/Berlin for this test to mean anything"
        );
        let anchor = utc("2026-03-01T00:00:00Z");
        let slot = c
            .next_slot_after(utc("2026-03-28T12:00:00Z"), anchor)
            .unwrap();
        // 03:00 CEST (+02:00) on the 29th — the first wall clock that exists.
        assert_eq!(slot, utc("2026-03-29T01:00:00Z"));
        let local = slot.with_timezone(&"Europe/Berlin".parse::<Tz>().unwrap());
        assert_eq!(
            local.date_naive().day(),
            29,
            "the occurrence is not dropped"
        );
        assert_eq!((local.hour(), local.minute()), (3, 0));
    }

    #[test]
    fn fall_back_takes_the_earlier_of_the_two_identical_wall_clocks() {
        // Europe/Berlin falls back on Sunday 25 October 2026: 03:00 → 02:00, so
        // 02:30 local happens twice — once at +02:00, once at +01:00.
        let c = cadence(serde_json::json!({
            "every": "day", "at": "02:30", "tz": "Europe/Berlin"
        }));
        // Same guard as the spring-forward test: prove the ambiguity is real,
        // so this cannot quietly degrade into asserting an ordinary day.
        let tz = "Europe/Berlin".parse::<Tz>().unwrap();
        let twice = NaiveDate::from_ymd_opt(2026, 10, 25)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();
        assert!(
            matches!(tz.from_local_datetime(&twice), LocalResult::Ambiguous(_, _)),
            "2026-10-25 02:30 must occur twice in Europe/Berlin for this test to mean anything"
        );
        let anchor = utc("2026-10-01T00:00:00Z");
        let slot = c
            .next_slot_after(utc("2026-10-24T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(
            slot,
            utc("2026-10-25T00:30:00Z"),
            "the earlier (still CEST) instant, so which one fires is deterministic"
        );
    }

    #[test]
    fn a_local_hour_survives_a_dst_boundary() {
        let c = cadence(serde_json::json!({
            "every": "week", "on": ["mon"], "at": "09:00", "tz": "Europe/Berlin"
        }));
        let tz = "Europe/Berlin".parse::<Tz>().unwrap();
        let anchor = utc("2026-01-01T00:00:00Z");
        // One Monday in winter (CET) and one in summer (CEST): different UTC
        // offsets, same local time. This is the whole reason for chrono-tz.
        let winter = c
            .next_slot_after(utc("2026-01-06T00:00:00Z"), anchor)
            .unwrap();
        let summer = c
            .next_slot_after(utc("2026-07-06T00:00:00Z"), anchor)
            .unwrap();
        assert_eq!(winter.with_timezone(&tz).hour(), 9);
        assert_eq!(summer.with_timezone(&tz).hour(), 9);
        assert_ne!(
            winter.hour(),
            summer.hour(),
            "the UTC hour must differ, or the zone is being ignored"
        );
    }

    // ---- month arithmetic --------------------------------------------------

    #[test]
    fn day_31_clamps_to_the_end_of_a_short_month() {
        let c = cadence(serde_json::json!({ "every": "month", "day": 31, "at": "10:00" }));
        let anchor = utc("2026-01-01T00:00:00Z");
        let feb = c
            .next_slot_after(utc("2026-02-01T00:00:00Z"), anchor)
            .unwrap();
        assert_eq!(feb, utc("2026-02-28T10:00:00Z"));
    }

    #[test]
    fn day_31_reaches_29_february_in_a_leap_year() {
        let c = cadence(serde_json::json!({ "every": "month", "day": 31, "at": "10:00" }));
        let anchor = utc("2028-01-01T00:00:00Z");
        let feb = c
            .next_slot_after(utc("2028-02-01T00:00:00Z"), anchor)
            .unwrap();
        assert_eq!(feb, utc("2028-02-29T10:00:00Z"));
    }

    #[test]
    fn month_defaults_to_the_first() {
        let c = cadence(serde_json::json!({ "every": "month", "at": "09:00" }));
        let anchor = utc("2026-01-01T00:00:00Z");
        let slot = c
            .next_slot_after(utc("2026-07-29T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-08-01T09:00:00Z"));
    }

    // ---- interval ----------------------------------------------------------

    #[test]
    fn fortnightly_counts_from_the_anchor_and_stays_put() {
        let c = cadence(serde_json::json!({
            "every": "week", "interval": 2, "on": ["mon"], "at": "09:00"
        }));
        // Anchor in the week of Monday 6 July 2026, so the 6th, 20th, 3 Aug…
        let anchor = utc("2026-07-06T00:00:00Z");
        let first = c
            .next_slot_after(utc("2026-07-06T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(first, utc("2026-07-20T09:00:00Z"), "skips the 13th");
        let second = c.next_slot_after(first, anchor).unwrap();
        assert_eq!(second, utc("2026-08-03T09:00:00Z"));
    }

    #[test]
    fn every_other_day_counts_from_the_anchor() {
        let c = cadence(serde_json::json!({
            "every": "day", "interval": 2, "at": "08:00"
        }));
        let anchor = utc("2026-07-01T00:00:00Z");
        // 1 July is on-cycle, so 3, 5, 7… and never the 2nd.
        let slot = c
            .next_slot_after(utc("2026-07-01T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-07-03T08:00:00Z"));
    }

    #[test]
    fn quarterly_is_a_month_interval() {
        let c = cadence(serde_json::json!({
            "every": "month", "interval": 3, "day": 1, "at": "10:00"
        }));
        let anchor = utc("2026-01-15T00:00:00Z");
        // Anchored in January, so January, April, July, October.
        let slot = c
            .next_slot_after(utc("2026-05-02T00:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-07-01T10:00:00Z"));
    }

    // ---- validation --------------------------------------------------------

    #[test]
    fn a_mistyped_field_is_refused_rather_than_ignored() {
        // The reason deny_unknown_fields is on: accepting this would drop the
        // weekday filter and fire every day instead of every Monday.
        let err = Cadence::parse(&serde_json::json!({
            "every": "week", "ony": ["mon"], "at": "09:00"
        }))
        .expect_err("an unknown field must not be silently ignored");
        assert!(
            err.contains("ony"),
            "the message should name the offending field, got: {err}"
        );
    }

    #[test]
    fn every_must_be_a_known_unit() {
        let err = Cadence::parse(&serde_json::json!({ "every": "fortnight", "at": "09:00" }))
            .expect_err("unknown unit");
        assert!(err.contains("day, week, month"), "got: {err}");
    }

    #[test]
    fn at_must_be_zero_padded_24_hour() {
        for bad in ["9:00", "09:00:00", "25:00", "09:60", "morning", ""] {
            let err = Cadence::parse(&serde_json::json!({ "every": "day", "at": bad }))
                .unwrap_err_or_default();
            assert!(
                err.contains("at must be"),
                "'{bad}' should be refused: {err}"
            );
        }
        assert!(parse_hhmm("00:00").is_some());
        assert!(parse_hhmm("23:59").is_some());
    }

    #[test]
    fn tz_must_be_an_iana_name() {
        let err = Cadence::parse(&serde_json::json!({
            "every": "day", "at": "09:00", "tz": "CEST"
        }))
        .expect_err("CEST is an abbreviation, not a zone");
        assert!(err.contains("IANA"), "got: {err}");
    }

    #[test]
    fn a_week_cadence_needs_weekdays() {
        let err = Cadence::parse(&serde_json::json!({ "every": "week", "at": "09:00" }))
            .expect_err("week without on");
        assert!(err.contains("weekday"), "got: {err}");
    }

    #[test]
    fn weekdays_on_a_day_cadence_are_an_error_not_an_extra() {
        // Silently ignoring `on` here would fire seven times a week when the
        // author asked for one.
        let err = Cadence::parse(&serde_json::json!({
            "every": "day", "on": ["mon"], "at": "09:00"
        }))
        .expect_err("on is meaningless on a day cadence");
        assert!(err.contains("every: week"), "got: {err}");
    }

    #[test]
    fn day_of_month_is_bounded() {
        let err = Cadence::parse(&serde_json::json!({
            "every": "month", "day": 32, "at": "09:00"
        }))
        .expect_err("32 is not a day");
        assert!(err.contains("between 1 and 31"), "got: {err}");
    }

    #[test]
    fn interval_is_bounded() {
        for bad in [0u32, MAX_INTERVAL + 1] {
            let err = Cadence::parse(&serde_json::json!({
                "every": "day", "at": "09:00", "interval": bad
            }))
            .expect_err("interval out of range");
            assert!(err.contains("interval must be"), "got: {err}");
        }
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let err = Cadence::parse(&serde_json::json!({
            "every": "fortnight", "at": "9am", "tz": "Mars/Olympus", "interval": 0
        }))
        .expect_err("four problems");
        for expected in ["every must be", "at must be", "IANA", "interval must be"] {
            assert!(err.contains(expected), "missing '{expected}' in: {err}");
        }
    }

    // ---- the cron teaching error ------------------------------------------

    #[test]
    fn a_cron_string_gets_translated_not_parsed() {
        let err =
            Cadence::parse(&serde_json::json!("0 9 * * mon")).expect_err("cron is not a cadence");
        assert!(err.contains(r#""every":"week""#), "got: {err}");
        assert!(err.contains(r#""on":["mon"]"#), "got: {err}");
        assert!(err.contains(r#""at":"09:00""#), "got: {err}");
        assert!(
            err.contains("tz"),
            "it must mention the missing zone: {err}"
        );
    }

    #[test]
    fn numeric_cron_weekdays_translate_too() {
        let err = Cadence::parse(&serde_json::json!("30 6 * * 0")).unwrap_err();
        assert!(err.contains(r#""on":["sun"]"#), "0 is Sunday: {err}");
        let err = Cadence::parse(&serde_json::json!("30 6 * * 7")).unwrap_err();
        assert!(err.contains(r#""on":["sun"]"#), "7 is Sunday too: {err}");
        let err = Cadence::parse(&serde_json::json!("0 9 * * 1,4")).unwrap_err();
        assert!(err.contains(r#""on":["mon","thu"]"#), "got: {err}");
    }

    #[test]
    fn cron_day_of_month_and_daily_translate() {
        let err = Cadence::parse(&serde_json::json!("0 9 1 * *")).unwrap_err();
        assert!(err.contains(r#""every":"month""#), "got: {err}");
        assert!(err.contains(r#""day":1"#), "got: {err}");
        let err = Cadence::parse(&serde_json::json!("30 6 * * *")).unwrap_err();
        assert!(err.contains(r#""every":"day""#), "got: {err}");
    }

    #[test]
    fn cron_we_cannot_express_says_so_instead_of_guessing() {
        let err = Cadence::parse(&serde_json::json!("*/15 * * * *")).unwrap_err();
        assert!(
            err.contains("cannot express"),
            "a stepped minute has no cadence equivalent: {err}"
        );
    }

    #[test]
    fn a_non_object_cadence_is_told_what_shape_to_use() {
        let err = Cadence::parse(&serde_json::json!(42)).unwrap_err();
        assert!(err.contains("must be an object"), "got: {err}");
        assert!(err.contains("every"), "got: {err}");
    }

    // ---- title placeholders ------------------------------------------------

    #[test]
    fn placeholders_render_in_the_cadence_zone() {
        let c = cadence(serde_json::json!({
            "every": "week", "on": ["mon"], "at": "09:00", "tz": "Europe/Berlin"
        }));
        let slot = utc("2026-08-03T07:00:00Z"); // Monday, 09:00 CEST
        assert_eq!(
            c.render("Weekly review — {week}", slot),
            "Weekly review — 2026-W32"
        );
        assert_eq!(c.render("{date}", slot), "2026-08-03");
        assert_eq!(c.render("{month}", slot), "2026-08");
        assert_eq!(c.render("{slot}", slot), "2026-08-03T07:00:00Z");
    }

    #[test]
    fn a_template_without_placeholders_is_left_alone() {
        let c = cadence(serde_json::json!({ "every": "day", "at": "09:00" }));
        let slot = utc("2026-08-03T09:00:00Z");
        assert_eq!(c.render("Verify the backup", slot), "Verify the backup");
    }

    #[test]
    fn a_local_date_can_differ_from_the_utc_one() {
        // 22:30 in Berlin is the next day in UTC; the label a human reads must
        // be the local one, or a nightly ticket is named for tomorrow.
        let c = cadence(serde_json::json!({
            "every": "day", "at": "23:30", "tz": "Europe/Berlin"
        }));
        let anchor = utc("2026-07-01T00:00:00Z");
        let slot = c
            .next_slot_after(utc("2026-07-29T12:00:00Z"), anchor)
            .unwrap();
        assert_eq!(slot, utc("2026-07-29T21:30:00Z"));
        assert_eq!(c.render("{date}", slot), "2026-07-29");
    }

    // ---- round-trip --------------------------------------------------------

    #[test]
    fn a_cadence_survives_a_json_round_trip() {
        // It is stored as JSON in schedules.spec, so this is the store's path.
        let c = cadence(serde_json::json!({
            "every": "week", "interval": 2, "on": ["mon", "thu"],
            "at": "09:00", "tz": "Europe/Berlin"
        }));
        let back = Cadence::parse(&serde_json::to_value(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn defaults_do_not_serialize_as_noise() {
        let c = cadence(serde_json::json!({ "every": "day", "at": "09:00" }));
        let json = serde_json::to_value(&c).unwrap();
        assert!(json.get("on").is_none(), "empty on should be omitted");
        assert!(json.get("day").is_none(), "unset day should be omitted");
        assert!(json.get("tz").is_none(), "unset tz should be omitted");
        assert_eq!(json["interval"], 1);
    }

    /// Small helper so the `at` loop above can assert on both the parse error
    /// and a `Cadence::parse` that unexpectedly succeeded.
    trait UnwrapErrOrDefault {
        fn unwrap_err_or_default(self) -> String;
    }
    impl UnwrapErrOrDefault for Result<Cadence, String> {
        fn unwrap_err_or_default(self) -> String {
            match self {
                Ok(c) => panic!("expected a rejection, got {c:?}"),
                Err(e) => e,
            }
        }
    }
}

//! Dialect shim: the seam the Postgres port moves through.
//!
//! # Why this exists
//!
//! The port is one-way — Postgres *instead of* SQLite — which sounds like it
//! removes the need for any abstraction. It does, at the END. It does not during
//! the move, and that distinction is the whole reason for this module.
//!
//! There is one `Store`. If `with_tx` hands its closure a
//! `&rusqlite::Transaction` today and a `postgres::Transaction` tomorrow, then
//! all 69 closures and the 48 private fns behind them must change in ONE commit
//! or the crate does not compile. That is a big-bang port: no test signal until
//! the very end, on a store whose failure modes (wrong row ordering) are silent.
//!
//! So the move goes through a scaffold, in two phases:
//!
//!   Phase 1  introduce this module, backed by rusqlite, and convert the store
//!            to it. Still SQLite underneath, so `tests/api.rs` and
//!            `tests/mcp.rs` stay green and any red is caused by the step that
//!            made it.
//!   Phase 2  swap what is behind this module for `postgres`. This file changes;
//!            the store does not.
//!
//! # The design constraint that makes phase 1 affordable
//!
//! Every method here mirrors the rusqlite method it replaces — same name, same
//! argument order, same return shape. That is not cosmetic. Closure parameters
//! in Rust are INFERRED, so `with_tx(|tx| tx.execute(...))` keeps compiling when
//! `tx` changes type, provided the methods still line up. It turns what looks
//! like 69 closure rewrites into a change of two signatures.
//!
//! [`Statement::query_map`] is the sharpest case: it returns an owned iterator
//! of results rather than a `Vec`, purely so the existing
//! `.query_map(..)?.collect::<Result<Vec<_>, _>>()` call sites survive untouched.
//! Postgres reads all rows eagerly, so behind the seam it is a `Vec` either way.
//!
//! # What it buys beyond sequencing
//!
//! Because the shim owns the SQL string on its way to the driver, it can rewrite
//! the dialect's mechanical differences in ONE place rather than at 714 call
//! sites — see [`to_pg_placeholders`].

use std::fmt;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// What a failure *was*, as far as the store is allowed to care.
///
/// The store branches on exactly three things, and each branch is a guarantee
/// rather than a nicety, so the classification has to survive the backend swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Anything the caller cannot act on: flattened to an opaque 500.
    Backend,
    /// A `query_row` that matched nothing. `OptionalExtension` turns this into
    /// `Ok(None)`; it must not be confused with a genuine failure.
    NoRows,
    /// A column could not be read as the requested type. `shares` relies on
    /// this: the `kind` column IS the enum, so an uninterpretable value must
    /// produce a teaching error rather than silently taking "the other branch".
    Conversion,
    /// A uniqueness/constraint violation. `schedules` relies on this for the
    /// exactly-once guarantee — one ticket per slot.
    Constraint,
}

/// A database error, backend-agnostic.
///
/// The message stays opaque to clients: `src/error.rs` flattens every database
/// failure to one generic message so raw SQL text never leaves the process, and
/// that property must not change with the backend.
#[derive(Debug)]
pub struct Error {
    msg: String,
    kind: Kind,
}

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error {
            msg: msg.into(),
            kind: Kind::Backend,
        }
    }
    pub fn with_kind(msg: impl Into<String>, kind: Kind) -> Self {
        Error {
            msg: msg.into(),
            kind,
        }
    }
    /// A column could not be read as the requested type.
    pub fn conversion(msg: impl Into<String>) -> Self {
        Error::with_kind(msg, Kind::Conversion)
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn is_conversion(&self) -> bool {
        self.kind == Kind::Conversion
    }

    /// True when this is a constraint violation whose message names `hint`.
    ///
    /// `schedules` uses it to tell the `(schedule, occurrence)` unique index
    /// apart from any other constraint — that index IS the exactly-once
    /// guarantee, so mistaking it for a generic failure would let a second
    /// ticket be created for one slot, or turn the correct refusal into a 500.
    ///
    /// **Phase 2 contract.** SQLite puts `table.column` in the message; Postgres
    /// puts the INDEX NAME. So the Postgres schema must name these indexes to
    /// contain the same substrings the store passes here — currently
    /// `tickets.occurrence` and `tickets.id`. That coupling is ugly and it is
    /// deliberate: the alternative is a silent behaviour change in the one place
    /// the schedule guarantee lives.
    pub fn is_constraint_on(&self, hint: &str) -> bool {
        self.kind == Kind::Constraint && self.msg.contains(hint)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        let kind = match &e {
            rusqlite::Error::QueryReturnedNoRows => Kind::NoRows,
            rusqlite::Error::FromSqlConversionFailure(..)
            | rusqlite::Error::InvalidColumnType(..) => Kind::Conversion,
            rusqlite::Error::SqliteFailure(f, _)
                if f.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Kind::Constraint
            }
            _ => Kind::Backend,
        };
        Error {
            msg: e.to_string(),
            kind,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Values and parameter binding
// ---------------------------------------------------------------------------

/// A dynamically-typed bound value, for SQL built at runtime (the ready queue,
/// ticket list filters, the event feed). Mirrors the `rusqlite::types::Value`
/// alias the store already used for this.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A type that can be bound as a query parameter.
pub trait ToSql {
    fn to_sql_value(&self) -> Value;
}

macro_rules! to_sql_via {
    ($t:ty, $v:expr) => {
        impl ToSql for $t {
            fn to_sql_value(&self) -> Value {
                #[allow(clippy::redundant_closure_call)]
                ($v)(self)
            }
        }
    };
}

to_sql_via!(i64, |s: &i64| Value::Integer(*s));
to_sql_via!(i32, |s: &i32| Value::Integer(i64::from(*s)));
to_sql_via!(u32, |s: &u32| Value::Integer(i64::from(*s)));
to_sql_via!(usize, |s: &usize| Value::Integer(*s as i64));
to_sql_via!(f64, |s: &f64| Value::Real(*s));
to_sql_via!(bool, |s: &bool| Value::Integer(i64::from(*s)));
to_sql_via!(String, |s: &String| Value::Text(s.clone()));
to_sql_via!(str, |s: &str| Value::Text(s.to_string()));
to_sql_via!(Vec<u8>, |s: &Vec<u8>| Value::Blob(s.clone()));
to_sql_via!([u8], |s: &[u8]| Value::Blob(s.to_vec()));

impl ToSql for Value {
    fn to_sql_value(&self) -> Value {
        self.clone()
    }
}

impl<T: ToSql + ?Sized> ToSql for &T {
    fn to_sql_value(&self) -> Value {
        (*self).to_sql_value()
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_sql_value(&self) -> Value {
        match self {
            Some(v) => v.to_sql_value(),
            None => Value::Null,
        }
    }
}

/// A bound parameter list. Implemented for what the store actually passes:
/// `params![..]`, an empty `[]`, and [`params_from_iter`].
pub trait Params {
    fn into_values(self) -> Vec<Value>;
}

impl Params for &[&dyn ToSql] {
    fn into_values(self) -> Vec<Value> {
        self.iter().map(|v| v.to_sql_value()).collect()
    }
}

impl<const N: usize> Params for [&dyn ToSql; N] {
    fn into_values(self) -> Vec<Value> {
        self.iter().map(|v| v.to_sql_value()).collect()
    }
}

// NOTE: no separate `impl<T: ToSql> Params for [T; 0]`. It would overlap the
// const-generic impl above at N = 0. The `[]` literal the store passes for "no
// parameters" resolves through that impl with N inferred as 0.

impl Params for Vec<Value> {
    fn into_values(self) -> Vec<Value> {
        self
    }
}

/// Bind a runtime-built parameter list. Mirrors `rusqlite::params_from_iter`.
pub fn params_from_iter<I>(iter: I) -> Vec<Value>
where
    I: IntoIterator,
    I::Item: ToSql,
{
    iter.into_iter().map(|v| v.to_sql_value()).collect()
}

/// Mirrors `rusqlite::params!`, producing a value this module's [`Params`]
/// accepts. Same call shape, so no call site changes.
macro_rules! params {
    () => { [] as [&dyn $crate::store::sql::ToSql; 0] };
    ($($v:expr),+ $(,)?) => {
        [$(&$v as &dyn $crate::store::sql::ToSql),+]
    };
}
pub(crate) use params;

// ---------------------------------------------------------------------------
// Reading rows
// ---------------------------------------------------------------------------

/// A type readable out of a result column.
pub trait FromSql: Sized {
    fn from_sql_value(v: &Value) -> Result<Self>;
}

impl FromSql for i64 {
    fn from_sql_value(v: &Value) -> Result<Self> {
        match v {
            Value::Integer(i) => Ok(*i),
            Value::Real(f) => Ok(*f as i64),
            other => Err(Error::new(format!("expected integer, got {other:?}"))),
        }
    }
}

impl FromSql for f64 {
    fn from_sql_value(v: &Value) -> Result<Self> {
        match v {
            Value::Real(f) => Ok(*f),
            Value::Integer(i) => Ok(*i as f64),
            other => Err(Error::new(format!("expected real, got {other:?}"))),
        }
    }
}

impl FromSql for bool {
    fn from_sql_value(v: &Value) -> Result<Self> {
        i64::from_sql_value(v).map(|i| i != 0)
    }
}

impl FromSql for String {
    fn from_sql_value(v: &Value) -> Result<Self> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            // SQLite is loosely typed and several columns hold integers that the
            // store reads as strings. Preserve that tolerance rather than making
            // the port fail on data it accepts today.
            Value::Integer(i) => Ok(i.to_string()),
            Value::Real(f) => Ok(f.to_string()),
            other => Err(Error::new(format!("expected text, got {other:?}"))),
        }
    }
}

impl FromSql for Vec<u8> {
    fn from_sql_value(v: &Value) -> Result<Self> {
        match v {
            Value::Blob(b) => Ok(b.clone()),
            Value::Text(s) => Ok(s.clone().into_bytes()),
            other => Err(Error::new(format!("expected blob, got {other:?}"))),
        }
    }
}

impl<T: FromSql> FromSql for Option<T> {
    fn from_sql_value(v: &Value) -> Result<Self> {
        match v {
            Value::Null => Ok(None),
            other => T::from_sql_value(other).map(Some),
        }
    }
}

/// One result row, addressable by column name or zero-based index — both forms
/// the store uses.
pub struct Row {
    names: std::rc::Rc<Vec<String>>,
    values: Vec<Value>,
}

/// How a column is addressed. Lets `row.get("id")` and `row.get(0)` coexist,
/// exactly as in rusqlite.
pub trait ColumnIndex {
    fn index_in(&self, names: &[String]) -> Result<usize>;
}

impl ColumnIndex for usize {
    fn index_in(&self, names: &[String]) -> Result<usize> {
        if *self < names.len() {
            Ok(*self)
        } else {
            Err(Error::new(format!("column index {self} out of range")))
        }
    }
}

impl ColumnIndex for &str {
    fn index_in(&self, names: &[String]) -> Result<usize> {
        names
            .iter()
            .position(|n| n == self)
            .ok_or_else(|| Error::new(format!("no such column: {self}")))
    }
}

impl Row {
    pub fn get<I: ColumnIndex, T: FromSql>(&self, idx: I) -> Result<T> {
        let i = idx.index_in(&self.names)?;
        T::from_sql_value(&self.values[i])
    }
}

// ---------------------------------------------------------------------------
// Statements, transactions, connections
// ---------------------------------------------------------------------------

/// A prepared statement. Holds the SQL and its executor; `query_map` mirrors
/// rusqlite's, returning an iterator of results so existing `.collect()` call
/// sites are unchanged.
pub struct Statement<'a> {
    inner: &'a rusqlite::Connection,
    sql: String,
}

impl Statement<'_> {
    pub fn query_map<P, T, F>(&mut self, params: P, f: F) -> Result<std::vec::IntoIter<Result<T>>>
    where
        P: Params,
        F: FnMut(&Row) -> Result<T>,
    {
        let rows = run_query(self.inner, &self.sql, params.into_values())?;
        let mut f = f;
        let mapped: Vec<Result<T>> = rows.iter().map(&mut f).collect();
        Ok(mapped.into_iter())
    }

    pub fn execute<P: Params>(&mut self, params: P) -> Result<usize> {
        run_execute(self.inner, &self.sql, params.into_values())
    }

    pub fn query_row<P, T, F>(&mut self, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T>,
    {
        let rows = run_query(self.inner, &self.sql, params.into_values())?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| Error::with_kind("query returned no rows", Kind::NoRows))?;
        f(&row)
    }
}

/// Shared execution against a rusqlite connection. The one place that knows
/// which backend is underneath — phase 2 replaces this function's body and the
/// two structs below, and nothing in `src/store/` moves.
fn run_query(conn: &rusqlite::Connection, sql: &str, params: Vec<Value>) -> Result<Vec<Row>> {
    let mut stmt = conn.prepare(sql)?;
    let names: std::rc::Rc<Vec<String>> = std::rc::Rc::new(
        stmt.column_names()
            .into_iter()
            .map(str::to_string)
            .collect(),
    );
    let n = names.len();
    let bound: Vec<rusqlite::types::Value> = params.into_iter().map(to_rusqlite).collect();
    let mut rows = stmt.query(rusqlite::params_from_iter(bound))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let mut values = Vec::with_capacity(n);
        for i in 0..n {
            values.push(from_rusqlite(r.get::<_, rusqlite::types::Value>(i)?));
        }
        out.push(Row {
            names: names.clone(),
            values,
        });
    }
    Ok(out)
}

fn run_execute(conn: &rusqlite::Connection, sql: &str, params: Vec<Value>) -> Result<usize> {
    let bound: Vec<rusqlite::types::Value> = params.into_iter().map(to_rusqlite).collect();
    Ok(conn.execute(sql, rusqlite::params_from_iter(bound))?)
}

fn to_rusqlite(v: Value) -> rusqlite::types::Value {
    match v {
        Value::Null => rusqlite::types::Value::Null,
        Value::Integer(i) => rusqlite::types::Value::Integer(i),
        Value::Real(f) => rusqlite::types::Value::Real(f),
        Value::Text(s) => rusqlite::types::Value::Text(s),
        Value::Blob(b) => rusqlite::types::Value::Blob(b),
    }
}

fn from_rusqlite(v: rusqlite::types::Value) -> Value {
    match v {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(i) => Value::Integer(i),
        rusqlite::types::Value::Real(f) => Value::Real(f),
        rusqlite::types::Value::Text(s) => Value::Text(s),
        rusqlite::types::Value::Blob(b) => Value::Blob(b),
    }
}

/// A read-only connection handle, as handed to `Store::with_conn`.
pub struct Conn<'a> {
    pub(super) inner: &'a rusqlite::Connection,
}

impl<'a> Conn<'a> {
    pub fn new(inner: &'a rusqlite::Connection) -> Self {
        Conn { inner }
    }
    /// Last auto-generated row id. Used for `events.seq`.
    pub fn last_insert_rowid(&self) -> i64 {
        self.inner.last_insert_rowid()
    }
    pub fn prepare(&self, sql: &str) -> Result<Statement<'a>> {
        Ok(Statement {
            inner: self.inner,
            sql: sql.to_string(),
        })
    }
    pub fn execute<P: Params>(&self, sql: &str, params: P) -> Result<usize> {
        run_execute(self.inner, sql, params.into_values())
    }
    pub fn query_row<P, T, F>(&self, sql: &str, params: P, f: F) -> Result<T>
    where
        P: Params,
        F: FnOnce(&Row) -> Result<T>,
    {
        let rows = run_query(self.inner, sql, params.into_values())?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| Error::with_kind("query returned no rows", Kind::NoRows))?;
        f(&row)
    }
}

/// A write transaction handle, as handed to `Store::with_tx`.
///
/// Derefs to [`Conn`], mirroring `rusqlite::Transaction: Deref<Target =
/// Connection>`. That is load-bearing, not convenience: dozens of helpers in
/// this store take `&Conn` for reading and are called with the transaction
/// during a write, relying on exactly that coercion. Without it every one of
/// those call sites would need a manual `.conn()`, which is 163 edits whose only
/// purpose is to restate what the type system already knew.
pub struct Tx<'a> {
    conn: Conn<'a>,
}

impl<'a> Tx<'a> {
    pub fn new(inner: &'a rusqlite::Transaction<'a>) -> Self {
        // `rusqlite::Transaction` derefs to `Connection`, so the transaction is
        // reachable as a connection and every method arrives through `Conn`.
        Tx {
            conn: Conn { inner },
        }
    }
}

impl<'a> std::ops::Deref for Tx<'a> {
    type Target = Conn<'a>;
    fn deref(&self) -> &Conn<'a> {
        &self.conn
    }
}

/// Mirrors `rusqlite::OptionalExtension`: turn "no rows" into `Ok(None)`.
pub trait OptionalExtension<T> {
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind == Kind::NoRows => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Dialect translation
// ---------------------------------------------------------------------------

/// Rewrite SQLite placeholders into Postgres ones.
///
/// SQLite accepts both `?` (positional, numbered by appearance) and `?N`
/// (explicit index, and freely REUSED — `?1` may appear three times). Postgres
/// accepts only `$N`, but reuse is fine there too, so both forms map cleanly:
///
/// ```text
///   WHERE a = ?1 AND b = ?2 AND c = ?1   ->   WHERE a = $1 AND b = $2 AND c = $1
///   WHERE a = ?  AND b = ?               ->   WHERE a = $1 AND b = $2
/// ```
///
/// # The sharp edge
///
/// A `?` inside a string literal is DATA, not a placeholder, and this store has
/// them: checklist coverage globs are matched with `GLOB`, and `?` is a
/// single-character wildcard in glob syntax, so `'src/?.rs'` reaches the
/// database as a literal. Rewriting that would corrupt the query into
/// `'src/$1.rs'` and change which files a lane claims to cover — silently, and
/// in the feature whose entire purpose is that coverage claims can be checked.
///
/// So the scanner tracks single-quoted string literals (including SQL's doubled
/// `''` escape) and double-quoted identifiers, and rewrites only outside them.
/// It also skips `--` line comments and `/* */` block comments.
///
/// Mixing bare `?` and `?N` in one statement is rejected rather than guessed at:
/// SQLite's numbering rule for the mixed case is subtle enough that a silent
/// reinterpretation is worse than a panic during a port.
// Unused by the store until phase 2 swaps the backend — it still runs on
// SQLite, which needs no rewriting. Must be `allow`, not `expect`: the tests
// below DO call it, so under `--all-targets` an `expect(dead_code)` is
// unfulfilled and clippy errors in the opposite direction.
#[allow(dead_code)] // scaffold: wired up when the backend swaps in phase 2
pub fn to_pg_placeholders(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut i = 0;
    let mut next_positional = 0usize;
    let mut saw_bare = false;
    let mut saw_numbered = false;

    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            // ---- string literal: copy verbatim, honouring the '' escape ----
            '\'' => {
                out.push('\'');
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        // A doubled quote is an escaped quote, not the end.
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            out.push_str("''");
                            i += 2;
                            continue;
                        }
                        out.push('\'');
                        i += 1;
                        break;
                    }
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- quoted identifier: copy verbatim ----
            '"' => {
                out.push('"');
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i] as char);
                    let end = bytes[i] == b'"';
                    i += 1;
                    if end {
                        break;
                    }
                }
            }
            // ---- line comment ----
            '-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- block comment ----
            '/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                out.push_str("/*");
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        out.push_str("*/");
                        i += 2;
                        break;
                    }
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- the placeholder itself ----
            '?' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if start == i {
                    saw_bare = true;
                    next_positional += 1;
                    out.push('$');
                    out.push_str(&next_positional.to_string());
                } else {
                    saw_numbered = true;
                    out.push('$');
                    out.push_str(&sql[start..i]);
                }
                assert!(
                    !(saw_bare && saw_numbered),
                    "SQL mixes bare ? and ?N placeholders, whose SQLite numbering \
                     is too subtle to translate silently. Make them all explicit. \
                     Offending SQL: {sql}"
                );
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_placeholders_map_directly_and_may_repeat() {
        assert_eq!(
            to_pg_placeholders("SELECT a FROM t WHERE x = ?1 AND y = ?2 AND z = ?1"),
            "SELECT a FROM t WHERE x = $1 AND y = $2 AND z = $1"
        );
    }

    #[test]
    fn bare_placeholders_are_numbered_by_appearance() {
        assert_eq!(
            to_pg_placeholders("SELECT a FROM t WHERE x = ? AND y = ? AND z = ?"),
            "SELECT a FROM t WHERE x = $1 AND y = $2 AND z = $3"
        );
    }

    /// The one that would silently change what a lane covers. `?` is a
    /// single-character wildcard in GLOB syntax, so it reaches the database as
    /// literal data inside a string.
    #[test]
    fn question_mark_inside_a_string_literal_is_data_not_a_placeholder() {
        assert_eq!(
            to_pg_placeholders("SELECT * FROM lane_globs WHERE glob GLOB 'src/?.rs' AND lane = ?1"),
            "SELECT * FROM lane_globs WHERE glob GLOB 'src/?.rs' AND lane = $1"
        );
    }

    #[test]
    fn doubled_quote_escape_does_not_end_the_literal_early() {
        assert_eq!(
            to_pg_placeholders("SELECT 'it''s ?' , ?1"),
            "SELECT 'it''s ?' , $1"
        );
    }

    #[test]
    fn placeholders_in_comments_are_left_alone() {
        assert_eq!(
            to_pg_placeholders("-- why ? here\nSELECT ?1"),
            "-- why ? here\nSELECT $1"
        );
        assert_eq!(
            to_pg_placeholders("/* a ? b */ SELECT ?1"),
            "/* a ? b */ SELECT $1"
        );
    }

    #[test]
    fn quoted_identifiers_are_left_alone() {
        assert_eq!(
            to_pg_placeholders(r#"SELECT "ref" FROM shares WHERE id = ?1"#),
            r#"SELECT "ref" FROM shares WHERE id = $1"#
        );
    }

    #[test]
    fn sql_without_placeholders_is_unchanged() {
        let sql = "SELECT COUNT(*) FROM tickets";
        assert_eq!(to_pg_placeholders(sql), sql);
    }

    #[test]
    #[should_panic(expected = "mixes bare ? and ?N")]
    fn mixing_the_two_forms_is_refused_rather_than_guessed() {
        to_pg_placeholders("SELECT a FROM t WHERE x = ?1 AND y = ?");
    }

    // ---- against SQL actually in this store, not invented examples ----

    /// `projects::QUESTIONS_OF_PROJECT`, verbatim. It reuses `?1` twice and is
    /// interpolated into a larger statement that also binds `?1` — the exact
    /// shape that would break a naive left-to-right renumbering translator.
    #[test]
    fn real_sql_reusing_one_index_across_an_interpolated_fragment() {
        let frag = "project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)";
        let sql = format!(
            "SELECT COUNT(*) FROM answer_grants WHERE project = ?1 OR question IN \
             (SELECT id FROM questions WHERE {frag})"
        );
        assert_eq!(
            to_pg_placeholders(&sql),
            "SELECT COUNT(*) FROM answer_grants WHERE project = $1 OR question IN \
             (SELECT id FROM questions WHERE project = $1 OR ticket IN \
             (SELECT id FROM tickets WHERE project = $1))"
        );
    }

    /// The ready-queue predicate from `claims::ready_scope`: built by appending
    /// fragments, bare `?` throughout.
    #[test]
    fn real_sql_from_the_dynamic_ready_queue_builder() {
        let sql = "SELECT x FROM tickets t \
                   WHERE (t.claim_holder IS NULL OR t.claim_expires_at <= ?) \
                   AND (t.expires_at IS NULL OR t.expires_at > ?) \
                   AND t.project = ? \
                   AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = ?) \
                   LIMIT ?";
        assert_eq!(
            to_pg_placeholders(sql),
            "SELECT x FROM tickets t \
             WHERE (t.claim_holder IS NULL OR t.claim_expires_at <= $1) \
             AND (t.expires_at IS NULL OR t.expires_at > $2) \
             AND t.project = $3 \
             AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = $4) \
             LIMIT $5"
        );
    }
}

//! `lattice account`: provision the credential files `latticed` reads (issue 582).
//!
//! A credential directory holds four owner-only files:
//!
//! | File | Read by | Content |
//! |---|---|---|
//! | `scram-verifiers` | `--scram-verifiers` | `account TAB SCRAM-SHA-256$4096:salt$StoredKey:ServerKey`, never a password |
//! | `scram-decoy-secret` | `--scram-decoy-secret` | 32 random bytes that make unknown accounts look like known ones |
//! | `account-policies` | `--account-policies` | `account TAB deny-all` or `account TAB tables: t=rw, ...` |
//! | `bearer-tokens` | `latticed --gateway-tokens` (ADR 0031) | salted token hashes, never a token |
//!
//! Secrets are never accepted on the command line and never logged: a password comes from stdin or
//! is generated, and is printed once on standard output; a token is generated and printed once.
//! Every file is created or replaced atomically with mode 0600 in a directory only the owner may
//! write. Every command that changes the directory first takes an exclusive lock on `DIR/.lock` and holds
//! it across its read-modify-write, so concurrent runs queue instead of writing back stale copies. The directory is meant to live outside the data directory so
//! that copying or backing up data does not copy credentials.

use lattice_api::auth::{
    Accounts, DecoySecret, MAX_ACCOUNTS, MAX_CREDENTIAL_BYTES, valid_account_name,
};
use lattice_api::bearer::{
    BearerToken, CredentialStore, STORE_HEADER, TokenId, TokenStatus, issue,
};
use lattice_api::exec_host::{Grant, MAX_POLICY_BYTES, PolicyTable};
use lattice_api::scram::Verifier;
use lattice_core::bugs::{self, Bug};
use lattice_seams::Rng;
use latticed::files::{create_private_new, read_private, replace_private};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// File names inside a credential directory.
const VERIFIERS: &str = "scram-verifiers";
const DECOY: &str = "scram-decoy-secret";
const POLICIES: &str = "account-policies";
const TOKENS: &str = "bearer-tokens";

/// The one SCRAM cost profile `latticed` accepts.
const ITERATIONS: u32 = 4096;
const SALT_BYTES: usize = 16;
/// Passwords: printable ASCII, so password preparation (SASLprep) is the identity.
const MIN_PASSWORD: usize = 12;
const MAX_PASSWORD: usize = 256;
/// A generated password: 24 random bytes as 48 hex digits (192 bits).
const GENERATED_PASSWORD_BYTES: usize = 24;
/// Rotation: the older token of an account ends this long after the newer one is created.
const DEFAULT_OVERLAP_HOURS: u64 = 24;
/// `token prune` removes dead entries at least this old unless told otherwise.
const DEFAULT_PRUNE_DAYS: u64 = 30;
const MAX_OVERLAP_HOURS: u64 = lattice_api::bearer::MAX_OVERLAP_SECS / 3600;

pub(crate) const HELP: &str = "\
usage: lattice account init DIR
       lattice account add DIR NAME [--read T1,T2] [--write T1] [--ddl T1] [--password-stdin]
       lattice account list DIR
       lattice account remove DIR NAME
       lattice account token create DIR NAME [--expires-days N] [--rotate [--overlap-hours N]]
       lattice account token revoke DIR TOKEN_ID
       lattice account token list DIR [NAME]
       lattice account token prune DIR [--older-than-days N]

Provisions the credential files that latticed reads, in the owner-only directory DIR (keep it
outside the data directory). `init` creates the 32-byte decoy secret, an empty token file and an
empty policy file, and prints the latticed flags. `add` writes a SCRAM verifier for NAME and its
grants: tables for --read, --write and --ddl (`*` means every table); with none the account is
denied everything. The password is read from stdin (--password-stdin, one line, 12 to 256 printable
ASCII characters) or generated; it is printed once and never stored. `token create` prints a bearer
token once; only a salted hash is stored. A second active token needs --rotate, which ends the older
one after --overlap-hours (default 24, at most 168). `remove` deletes the account and revokes its
tokens. Passwords and tokens are never accepted as arguments. Files are created with mode 0600 and
existing files are never overwritten by `init`. `token prune` deletes revoked and expired entries older than N days (default 30; the store holds at most 1024 entries). Commands that change the directory take a lock on DIR/.lock and wait for each other. A running latticed with `--gateway-tokens` reads a changed bearer-token file within 5 seconds, so a revoke or a remove closes that account's live gateway connections without a restart. SCRAM verifiers and account policies are read at start only: restart latticed after a password change, an account add or an account remove. SCRAM verifiers use 4096 PBKDF2 iterations, so a stolen verifier file is cheap to attack offline: protect it like a key and use long random passwords (the generated ones are). --password-stdin refuses a terminal because it would echo; pipe the password in.
";

/// Everything a command needs from outside: stdin, randomness, the clock.
pub(crate) struct Env<'a> {
    pub stdin: &'a mut dyn Read,
    pub rng: &'a mut dyn Rng,
    pub now_secs: u64,
    /// Whether `stdin` is a terminal, where a typed password would echo.
    pub stdin_is_terminal: bool,
}

/// Runs `lattice account ARGS` and returns the standard output text.
pub(crate) fn run(args: &[String], env: &mut Env<'_>) -> Result<Zeroizing<String>, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["--help" | "-h" | "help"] => Ok(Zeroizing::new(HELP.to_string())),
        ["init", rest @ ..] => {
            let p = parse(rest, &[], 1, 1)?;
            init(&dir_of(&p, 0)?, env)
        }
        ["add", rest @ ..] => {
            let p = parse(
                rest,
                &[
                    ("--read", true),
                    ("--write", true),
                    ("--ddl", true),
                    ("--password-stdin", false),
                ],
                2,
                2,
            )?;
            let grants = Grants {
                read: tables_of(&p, "--read")?,
                write: tables_of(&p, "--write")?,
                ddl: tables_of(&p, "--ddl")?,
            };
            add(
                &dir_of(&p, 0)?,
                positional(&p, 1)?,
                &grants,
                p.flag("--password-stdin"),
                env,
            )
        }
        ["list", rest @ ..] => {
            let p = parse(rest, &[], 1, 1)?;
            list(&dir_of(&p, 0)?, env.now_secs)
        }
        ["remove", rest @ ..] => {
            let p = parse(rest, &[], 2, 2)?;
            remove(&dir_of(&p, 0)?, positional(&p, 1)?)
        }
        ["token", "create", rest @ ..] => {
            let p = parse(
                rest,
                &[
                    ("--expires-days", true),
                    ("--rotate", false),
                    ("--overlap-hours", true),
                ],
                2,
                2,
            )?;
            let expires_days = number_flag(&p, "--expires-days", false)?;
            let overlap = number_flag(&p, "--overlap-hours", false)?;
            if overlap.is_some() && !p.flag("--rotate") {
                return Err("--overlap-hours needs --rotate".into());
            }
            token_create(
                &dir_of(&p, 0)?,
                positional(&p, 1)?,
                expires_days,
                p.flag("--rotate")
                    .then(|| overlap.unwrap_or(DEFAULT_OVERLAP_HOURS)),
                env,
            )
        }
        ["token", "revoke", rest @ ..] => {
            let p = parse(rest, &[], 2, 2)?;
            token_revoke(&dir_of(&p, 0)?, positional(&p, 1)?)
        }
        ["token", "prune", rest @ ..] => {
            let p = parse(rest, &[("--older-than-days", true)], 1, 1)?;
            let days = number_flag(&p, "--older-than-days", true)?.unwrap_or(DEFAULT_PRUNE_DAYS);
            token_prune(&dir_of(&p, 0)?, days, env)
        }
        ["token", "list", rest @ ..] => {
            let p = parse(rest, &[], 1, 2)?;
            token_list(&dir_of(&p, 0)?, p.positional.get(1).copied(), env.now_secs)
        }
        _ => Err("unknown account command (see `lattice account --help`)".into()),
    }
}

// ---- argument parsing -----------------------------------------------------------------------

struct Parsed<'a> {
    positional: Vec<&'a str>,
    flags: BTreeMap<&'a str, Option<&'a str>>,
}
impl Parsed<'_> {
    fn flag(&self, name: &str) -> bool {
        self.flags.contains_key(name)
    }
    fn value(&self, name: &str) -> Option<&str> {
        self.flags.get(name).copied().flatten()
    }
}

/// Splits `args` into positionals and the listed flags. An unknown flag, a repeated flag or a
/// missing value is an error that never repeats the offending value (it could be a secret).
fn parse<'a>(
    args: &[&'a str],
    allowed: &[(&'static str, bool)],
    min: usize,
    max: usize,
) -> Result<Parsed<'a>, String> {
    let mut positional = Vec::new();
    let mut flags = BTreeMap::new();
    let mut iter = args.iter().copied();
    while let Some(word) = iter.next() {
        if !word.starts_with("--") {
            positional.push(word);
            continue;
        }
        let name = word.split('=').next().unwrap_or(word);
        let Some(&(known, takes_value)) = allowed.iter().find(|(n, _)| *n == name) else {
            return Err(format!("unknown option {name}"));
        };
        if word.len() != name.len() {
            return Err(format!("write {known} VALUE, not {known}=VALUE"));
        }
        let value = if takes_value {
            Some(
                iter.next()
                    .ok_or_else(|| format!("{known} needs a value"))?,
            )
        } else {
            None
        };
        if flags.insert(known, value).is_some() {
            return Err(format!("{known} given twice"));
        }
    }
    if positional.len() < min || positional.len() > max {
        return Err("wrong number of arguments (see `lattice account --help`)".into());
    }
    Ok(Parsed { positional, flags })
}

fn positional<'a>(p: &Parsed<'a>, index: usize) -> Result<&'a str, String> {
    p.positional
        .get(index)
        .copied()
        .ok_or_else(|| "missing argument".to_string())
}

fn dir_of(p: &Parsed<'_>, index: usize) -> Result<Dir, String> {
    Ok(Dir(PathBuf::from(positional(p, index)?)))
}

fn number_flag(p: &Parsed<'_>, name: &str, allow_zero: bool) -> Result<Option<u64>, String> {
    p.value(name)
        .map(|v| {
            v.parse::<u64>()
                .ok()
                .filter(|n| allow_zero || *n > 0)
                .ok_or_else(|| format!("{name} needs a positive whole number"))
        })
        .transpose()
}

fn tables_of(p: &Parsed<'_>, name: &str) -> Result<Vec<String>, String> {
    match p.value(name) {
        None => Ok(Vec::new()),
        Some(list) => Ok(list.split(',').map(str::to_string).collect()),
    }
}

// ---- the credential directory ---------------------------------------------------------------

struct Dir(PathBuf);
impl Dir {
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

fn io_error(path: &Path, e: &std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

/// The text of a credential file, or `None` when it does not exist.
fn read_text(path: &Path, limit: usize) -> Result<Option<Zeroizing<String>>, String> {
    match read_private(path, limit) {
        // The verifier file holds key material: keep it in wiped buffers.
        Ok(bytes) => std::str::from_utf8(&bytes)
            .map(|text| Some(Zeroizing::new(text.to_string())))
            .map_err(|_| format!("{}: not UTF-8", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path, &e)),
    }
}

/// A file `init` must have created.
fn require_text(dir: &Dir, name: &str, limit: usize) -> Result<Zeroizing<String>, String> {
    read_text(&dir.file(name), limit)?.ok_or_else(|| {
        format!(
            "{} is missing: run `lattice account init {}` first",
            dir.file(name).display(),
            dir.0.display()
        )
    })
}

fn load_tokens(dir: &Dir) -> Result<CredentialStore, String> {
    let path = dir.file(TOKENS);
    latticed::load_credential_store(&path).map_err(|e| io_error(&path, &e))
}

fn save_tokens(dir: &Dir, store: &CredentialStore) -> Result<(), String> {
    let path = dir.file(TOKENS);
    latticed::write_credential_store(&path, store).map_err(|e| io_error(&path, &e))
}

fn save(dir: &Dir, name: &str, text: &str) -> Result<(), String> {
    let path = dir.file(name);
    replace_private(&path, text.as_bytes()).map_err(|e| io_error(&path, &e))
}

/// Takes the directory lock for a command that changes the directory. Hold the returned file
/// until the command has written everything.
fn lock(dir: &Dir) -> Result<std::fs::File, String> {
    latticed::files::lock_dir(&dir.0).map_err(|e| io_error(&dir.file(".lock"), &e))
}

/// `account TAB rest` lines of a verifier or policy file, in file order.
fn lines_of(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .collect()
}

fn without_account(text: &str, name: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::new());
    for (account, rest) in lines_of(text) {
        if account != name {
            let _ = writeln!(out, "{account}\t{rest}");
        }
    }
    out
}

/// A `latticed` accepts only names the shared account rule accepts.
fn check_name(name: &str) -> Result<(), String> {
    if valid_account_name(name) {
        Ok(())
    } else {
        Err("account names are 1 to 63 bytes without control characters".into())
    }
}

fn account_names(verifiers: &str, policies: &str) -> Vec<String> {
    let mut names: Vec<String> = lines_of(verifiers)
        .into_iter()
        .chain(lines_of(policies))
        .map(|(n, _)| n.to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

// ---- secrets --------------------------------------------------------------------------------

fn random_bytes(rng: &mut (impl Rng + ?Sized), out: &mut [u8]) {
    for chunk in out.chunks_mut(8) {
        let word = rng.next_u64().to_le_bytes();
        for (slot, byte) in chunk.iter_mut().zip(word) {
            *slot = byte;
        }
    }
}

/// The credential-file line of a prepared password: `SCRAM-SHA-256$4096:salt$StoredKey:ServerKey`.
pub(crate) fn scram_line(password: &[u8], salt: &[u8]) -> Result<Zeroizing<String>, String> {
    Verifier::from_prepared_password(password, salt, ITERATIONS)
        .map(|v| v.to_line())
        .map_err(|_| "cannot derive a SCRAM verifier from this password".to_string())
}

/// Accepts printable ASCII of 12 to 256 bytes. SASLprep is the identity on those, so the bytes the
/// verifier is derived from are the bytes a client sends.
fn check_password(password: &str) -> Result<(), String> {
    if !weak_passwords_allowed() && password.len() < MIN_PASSWORD {
        return Err(format!(
            "a password needs at least {MIN_PASSWORD} characters"
        ));
    }
    if password.is_empty() || password.len() > MAX_PASSWORD {
        return Err(format!(
            "a password needs at most {MAX_PASSWORD} characters"
        ));
    }
    if !password.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err("a password may hold printable ASCII characters only".into());
    }
    Ok(())
}

/// Planted defect: the minimum length is not enforced.
fn weak_passwords_allowed() -> bool {
    bugs::active(Bug::WeakPasswordAccepted)
}

/// One line from `stdin`, without its line ending. Reads at most one byte past the limit.
fn read_password(stdin: &mut dyn Read) -> Result<Zeroizing<String>, String> {
    let mut bytes = Zeroizing::new(Vec::new());
    stdin
        .take(MAX_PASSWORD as u64 + 3)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read the password from stdin".to_string())?;
    let line = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let text =
        String::from_utf8(line.to_vec()).map_err(|_| "the password is not UTF-8".to_string())?;
    Ok(Zeroizing::new(text))
}

fn hex(bytes: &[u8]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(bytes.len() * 2));
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

// ---- commands -------------------------------------------------------------------------------

fn flags_text(dir: &Dir) -> String {
    let absolute = std::path::absolute(&dir.0).unwrap_or_else(|_| dir.0.clone());
    let at = |name: &str| absolute.join(name).display().to_string();
    format!(
        "latticed flags:\n  --auth scram-tls --scram-verifiers {} --scram-decoy-secret {} \\\n  --account-policies {} --tls-cert CERT.pem --tls-key KEY.pem\nbearer tokens: {} (latticed --gateway-tokens; reloaded within 5 seconds)\n",
        at(VERIFIERS),
        at(DECOY),
        at(POLICIES),
        at(TOKENS),
    )
}

fn init(dir: &Dir, env: &mut Env<'_>) -> Result<Zeroizing<String>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        if !dir.0.exists() {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&dir.0)
                .map_err(|e| io_error(&dir.0, &e))?;
        }
    }
    let _lock = lock(dir)?;
    // Refuse before writing anything, so a refused init leaves no half-provisioned directory.
    for name in [DECOY, TOKENS, POLICIES, VERIFIERS] {
        if dir.file(name).symlink_metadata().is_ok() {
            return Err(format!(
                "{} already exists; init never overwrites a credential file",
                dir.file(name).display()
            ));
        }
    }
    let mut secret = Zeroizing::new([0u8; 32]);
    random_bytes(&mut *env.rng, secret.as_mut());
    let header = format!("{STORE_HEADER}\n");
    create_all(
        dir,
        &[
            (DECOY, secret.as_ref()),
            (POLICIES, b""),
            (TOKENS, header.as_bytes()),
        ],
    )?;
    Ok(Zeroizing::new(format!(
        "initialized {}\nThe verifier file appears with the first `account add`; latticed refuses an empty one.\n{}",
        dir.0.display(),
        flags_text(dir)
    )))
}

/// Creates every file or none: when one fails, the ones this call created are removed again, so a
/// retry starts from the directory it was given.
fn create_all(dir: &Dir, files: &[(&str, &[u8])]) -> Result<(), String> {
    let mut created: Vec<PathBuf> = Vec::new();
    for (name, bytes) in files {
        let path = dir.file(name);
        match create_private_new(&path, bytes) {
            Ok(()) => created.push(path),
            Err(e) => {
                for done in &created {
                    let _ = std::fs::remove_file(done);
                }
                return Err(io_error(&path, &e));
            }
        }
    }
    Ok(())
}

struct Grants {
    read: Vec<String>,
    write: Vec<String>,
    ddl: Vec<String>,
}
impl Grants {
    /// The policy column: `deny-all` without grants, else `tables: a=rw, b=r` in name order.
    fn policy(&self) -> Result<String, String> {
        for name in self.read.iter().chain(&self.write).chain(&self.ddl) {
            check_table_name(name)?;
        }
        let mut bits: BTreeMap<&str, [bool; 3]> = BTreeMap::new();
        for (slot, names) in [&self.read, &self.write, &self.ddl].into_iter().enumerate() {
            for name in names {
                if let Some(flags) = bits.entry(name.as_str()).or_default().get_mut(slot) {
                    *flags = true;
                }
            }
        }
        if bits.is_empty() {
            return Ok("deny-all".to_string());
        }
        let mut text = String::from("tables: ");
        for (i, (name, [r, w, d])) in bits.iter().enumerate() {
            if i > 0 {
                text.push_str(", ");
            }
            let mut b = String::new();
            for (on, c) in [(*r, 'r'), (*w, 'w'), (*d, 'd')] {
                if on {
                    b.push(c);
                }
            }
            let _ = write!(text, "{name}={b}");
        }
        // The line must stay one line of one account, whatever the names held.
        if text.chars().any(char::is_control) {
            return Err("the grants hold a control character".into());
        }
        Grant::parse_account(&text).map_err(|e| format!("the grants are invalid: {e:?}"))?;
        Ok(text)
    }
}

/// A table name goes into a tab- and line-separated file, so it is checked before it is composed
/// into a line: no control character or whitespace (a newline or tab would start another account's
/// line), no `=` or `,` (they delimit grants), and the shared grant parser must accept it alone.
fn check_table_name(name: &str) -> Result<(), String> {
    let bad = name.is_empty()
        || name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '=' || c == ',');
    if bad || Grant::parse_account(&format!("tables: {name}=r")).is_err() {
        return Err("a table name is invalid (no spaces, control characters, `=` or `,`)".into());
    }
    Ok(())
}

fn add(
    dir: &Dir,
    name: &str,
    grants: &Grants,
    password_from_stdin: bool,
    env: &mut Env<'_>,
) -> Result<Zeroizing<String>, String> {
    check_name(name)?;
    if password_from_stdin && env.stdin_is_terminal {
        return Err(
            "stdin is a terminal and would echo the password: pipe it in, or omit --password-stdin to generate one"
                .into(),
        );
    }
    let policy = grants.policy()?;
    let _lock = lock(dir)?;
    let verifiers = read_text(&dir.file(VERIFIERS), MAX_CREDENTIAL_BYTES)?.unwrap_or_default();
    let policies = require_text(dir, POLICIES, MAX_POLICY_BYTES)?;
    require_text(dir, TOKENS, lattice_api::bearer::MAX_STORE_BYTES)?;
    if account_names(&verifiers, &policies)
        .iter()
        .any(|n| n == name)
    {
        return Err("that account already exists (remove it first)".into());
    }
    let generated = !password_from_stdin;
    let password = if password_from_stdin {
        read_password(&mut *env.stdin)?
    } else {
        let mut raw = Zeroizing::new([0u8; GENERATED_PASSWORD_BYTES]);
        random_bytes(&mut *env.rng, raw.as_mut());
        hex(raw.as_ref())
    };
    check_password(&password)?;
    let mut salt = [0u8; SALT_BYTES];
    random_bytes(&mut *env.rng, &mut salt);
    let line = scram_line(password.as_bytes(), &salt)?;

    let mut new_verifiers = verifiers;
    let _ = writeln!(new_verifiers, "{name}\t{}", *line);
    // What latticed would do with the files: both must load, or nothing is written.
    let decoy = DecoySecret::from_bytes(&[0; 32]).map_err(|_| "internal error".to_string())?;
    Accounts::parse(&new_verifiers, decoy).map_err(|_| {
        format!("the verifier file would be invalid (at most {MAX_ACCOUNTS} accounts)")
    })?;
    let mut new_policies = policies.to_string();
    let _ = writeln!(new_policies, "{name}\t{policy}");
    PolicyTable::parse_accounts(&new_policies)
        .map_err(|e| format!("the grants are invalid: {e:?}"))?;

    // Policy first: an account is only able to log in once its verifier exists, and until then its
    // grants deny nothing that matters. A failure in between leaves a grant without a login.
    save(dir, POLICIES, &new_policies)?;
    save(dir, VERIFIERS, &new_verifiers)?;
    let mut out = Zeroizing::new(format!("account {name} added\ngrants: {policy}\n"));
    if generated {
        let _ = writeln!(out, "password (shown once, not stored): {}", *password);
    } else {
        out.push_str("password read from stdin (not stored)\n");
    }
    Ok(out)
}

/// Active and not past its expiry: the tokens that can authenticate.
fn is_live(e: &lattice_api::bearer::TokenEntry, now: u64) -> bool {
    e.status() == TokenStatus::Active && e.expires().is_none_or(|at| now < at)
}

fn list(dir: &Dir, env_now: u64) -> Result<Zeroizing<String>, String> {
    let verifiers = read_text(&dir.file(VERIFIERS), MAX_CREDENTIAL_BYTES)?.unwrap_or_default();
    let policies = require_text(dir, POLICIES, MAX_POLICY_BYTES)?;
    let tokens = load_tokens(dir)?;
    let mut out = Zeroizing::new(String::new());
    let names = account_names(&verifiers, &policies);
    for name in &names {
        let scram = lines_of(&verifiers).iter().any(|(n, _)| n == name);
        let grants = lines_of(&policies)
            .into_iter()
            .find(|(n, _)| n == name)
            .map_or("(none: denied)", |(_, g)| g);
        let active = tokens
            .entries()
            .filter(|e| e.account() == name && is_live(e, env_now))
            .count();
        let _ = writeln!(
            out,
            "{name}\tscram: {}\ttokens active: {active}\tgrants: {grants}",
            if scram { "yes" } else { "no" }
        );
    }
    let _ = writeln!(out, "{} accounts", names.len());
    Ok(out)
}

fn remove(dir: &Dir, name: &str) -> Result<Zeroizing<String>, String> {
    check_name(name)?;
    let _lock = lock(dir)?;
    let verifiers = read_text(&dir.file(VERIFIERS), MAX_CREDENTIAL_BYTES)?.unwrap_or_default();
    let policies = require_text(dir, POLICIES, MAX_POLICY_BYTES)?;
    let mut tokens = load_tokens(dir)?;
    let known = account_names(&verifiers, &policies)
        .iter()
        .any(|n| n == name)
        || tokens.entries().any(|e| e.account() == name);
    if !known {
        return Err("no such account".into());
    }
    // Stop access first (tokens), then the login, then the grants.
    let revoked = tokens.revoke_account(name);
    save_tokens(dir, &tokens)?;
    let remaining = without_account(&verifiers, name);
    if remaining.is_empty() {
        // latticed refuses an empty verifier file, so the file goes with its last account.
        let path = dir.file(VERIFIERS);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&path, &e)),
        }
    } else {
        save(dir, VERIFIERS, &remaining)?;
    }
    save(dir, POLICIES, &without_account(&policies, name))?;
    Ok(Zeroizing::new(format!(
        "account {name} removed; {revoked} bearer tokens revoked\n"
    )))
}

fn token_create(
    dir: &Dir,
    name: &str,
    expires_days: Option<u64>,
    rotate_overlap_hours: Option<u64>,
    env: &mut Env<'_>,
) -> Result<Zeroizing<String>, String> {
    check_name(name)?;
    require_clock(env.now_secs)?;
    let _lock = lock(dir)?;
    let verifiers = read_text(&dir.file(VERIFIERS), MAX_CREDENTIAL_BYTES)?.unwrap_or_default();
    let policies = require_text(dir, POLICIES, MAX_POLICY_BYTES)?;
    if !account_names(&verifiers, &policies)
        .iter()
        .any(|n| n == name)
    {
        return Err("no such account (add it first)".into());
    }
    let mut store = load_tokens(dir)?;
    let now = env.now_secs;
    // Expired tokens cannot authenticate; revoking them frees their slot in the two-active rule.
    store.revoke_expired(now);
    if store.len() >= lattice_api::bearer::MAX_ENTRIES {
        store.prune(now, DEFAULT_PRUNE_DAYS * 86_400);
    }
    if store.len() >= lattice_api::bearer::MAX_ENTRIES {
        return Err(
            "the token file is full of recent entries: run `lattice account token prune DIR --older-than-days 0`"
                .into(),
        );
    }
    let expires = match expires_days {
        None => None,
        Some(days) => Some(
            days.checked_mul(86_400)
                .and_then(|s| now.checked_add(s))
                .ok_or("--expires-days is too large")?,
        ),
    };
    let active: Vec<TokenId> = store
        .entries()
        .filter(|e| e.account() == name && e.status() == TokenStatus::Active)
        .map(|e| e.id())
        .collect();
    match (active.as_slice(), rotate_overlap_hours) {
        ([], _) => {}
        ([old], Some(hours)) => {
            if hours > MAX_OVERLAP_HOURS {
                return Err(format!("--overlap-hours is at most {MAX_OVERLAP_HOURS}"));
            }
            let end = now + hours * 3600;
            // Never lengthen the older token's life.
            let end = store
                .get(*old)
                .and_then(|e| e.expires())
                .map_or(end, |existing| existing.min(end));
            store.set_expiry(*old, end).map_err(|e| e.to_string())?;
        }
        ([_], None) => {
            return Err(
                "that account already has an active token; pass --rotate to issue a second one and end the first after the overlap"
                    .into(),
            );
        }
        _ => {
            return Err("that account already has two active tokens; revoke one first".into());
        }
    }
    let (token, entry) = issue(name, now, expires, &mut *env.rng).map_err(|e| e.to_string())?;
    let id = entry.id();
    store.insert(entry).map_err(|e| e.to_string())?;
    save_tokens(dir, &store)?;
    Ok(Zeroizing::new(format!(
        "token {id} created for {name}\ntoken (shown once, only a salted hash is stored): {}\n",
        show(&token)
    )))
}

/// A clock reading of 0 would issue tokens created at the epoch, which no host would trust.
fn require_clock(now_secs: u64) -> Result<(), String> {
    if now_secs == 0 {
        return Err(
            "the system clock reads 1970-01-01: set the clock before issuing tokens".into(),
        );
    }
    Ok(())
}

fn show(token: &BearerToken) -> &str {
    token.expose_secret()
}

fn token_revoke(dir: &Dir, id: &str) -> Result<Zeroizing<String>, String> {
    let id = TokenId::parse(id).ok_or("a token id is 16 lowercase hex digits")?;
    let _lock = lock(dir)?;
    let mut store = load_tokens(dir)?;
    if !store.revoke(id) {
        return Err("no such token".into());
    }
    save_tokens(dir, &store)?;
    Ok(Zeroizing::new(format!("token {id} revoked\n")))
}

fn token_prune(dir: &Dir, days: u64, env: &Env<'_>) -> Result<Zeroizing<String>, String> {
    require_clock(env.now_secs)?;
    let _lock = lock(dir)?;
    let mut store = load_tokens(dir)?;
    let secs = days
        .checked_mul(86_400)
        .ok_or("--older-than-days is too large")?;
    let removed = store.prune(env.now_secs, secs);
    save_tokens(dir, &store)?;
    Ok(Zeroizing::new(format!(
        "{removed} dead tokens removed, {} entries remain\n",
        store.len()
    )))
}

fn token_list(dir: &Dir, account: Option<&str>, now: u64) -> Result<Zeroizing<String>, String> {
    let store = load_tokens(dir)?;
    let mut out = Zeroizing::new(String::new());
    let mut count = 0;
    for e in store
        .entries()
        .filter(|e| account.is_none_or(|a| e.account() == a))
    {
        count += 1;
        let status = match e.status() {
            TokenStatus::Active if is_live(e, now) => "active",
            TokenStatus::Active => "expired",
            TokenStatus::Revoked => "revoked",
        };
        let expires = e
            .expires()
            .map_or_else(|| "never".to_string(), |t| t.to_string());
        let _ = writeln!(
            out,
            "{}\t{}\t{status}\tcreated {}\texpires {expires}",
            e.id(),
            e.account(),
            e.created()
        );
    }
    let _ = writeln!(out, "{count} tokens");
    Ok(out)
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use lattice_api::bearer::{AuthError, RejectReason, verify_bearer};
    use std::io::Cursor;
    use std::os::unix::fs::PermissionsExt;

    struct Counter(u64);
    impl Rng for Counter {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            self.0
        }
    }

    const NOW: u64 = 1_800_000_000;
    const PASSWORD: &str = "correct horse battery staple";

    fn directory(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lattice-account-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Runs argument words (no splitting, so a word may hold a tab or newline) at time `now`.
    fn run_at(
        args: &[&str],
        stdin: &str,
        seed: u64,
        now: u64,
        terminal: bool,
    ) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let mut input = Cursor::new(stdin.as_bytes().to_vec());
        let mut rng = Counter(seed);
        run(
            &args,
            &mut Env {
                stdin: &mut input,
                rng: &mut rng,
                now_secs: now,
                stdin_is_terminal: terminal,
            },
        )
        .map(|text| text.to_string())
    }

    /// Runs a command line given as one string, with `stdin` and a fresh seeded generator.
    fn cmd(line: &str, stdin: &str, seed: u64) -> Result<String, String> {
        cmd_at(line, stdin, seed, NOW)
    }

    fn cmd_at(line: &str, stdin: &str, seed: u64, now: u64) -> Result<String, String> {
        let words: Vec<&str> = line.split_whitespace().collect();
        run_at(&words, stdin, seed, now, false)
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn ready(name: &str) -> (PathBuf, String) {
        let dir = directory(name);
        let d = dir.display().to_string();
        cmd(&format!("init {d}"), "", 1).unwrap();
        (dir, d)
    }

    #[test]
    fn scram_line_matches_independently_computed_vectors() {
        // RFC 7677 section 3: user/pencil, salt W22ZaJ0SNY7soEsUEjb6gQ==, 4096 iterations.
        let salt = [
            0x5b, 0x6d, 0x99, 0x68, 0x9d, 0x12, 0x35, 0x8e, 0xec, 0xa0, 0x4b, 0x14, 0x12, 0x36,
            0xfa, 0x81,
        ];
        assert_eq!(
            &**scram_line(b"pencil", &salt).unwrap(),
            "SCRAM-SHA-256$4096:W22ZaJ0SNY7soEsUEjb6gQ==$WG5d8oPm3OtcPnkdi4Uo7BkeZkBFzpcXkuLmtbsT4qY=:wfPLwcE6nTWhTAmQ7tl2KeoiWGPlZqQxSrmfPwDl2dU="
        );
        // Computed with Python hashlib.pbkdf2_hmac / hmac, outside this crate.
        let salt: Vec<u8> = (0..16).collect();
        assert_eq!(
            &**scram_line(PASSWORD.as_bytes(), &salt).unwrap(),
            "SCRAM-SHA-256$4096:AAECAwQFBgcICQoLDA0ODw==$ONYbSJBXtKl6bP6PVqw8pm9e7EiacprLnoUQPFS80Hw=:IPOtHuGJ2HifEQg74W2XXqqCrCyQG55GbPRHa6g6n9w="
        );
    }

    #[test]
    fn init_creates_private_files_prints_flags_and_refuses_overwrite() {
        let (dir, d) = ready("init");
        assert_eq!(mode(&dir), 0o700);
        for name in [DECOY, TOKENS, POLICIES] {
            assert_eq!(mode(&dir.join(name)), 0o600, "{name}");
        }
        assert!(!dir.join(VERIFIERS).exists());
        assert_eq!(std::fs::read(dir.join(DECOY)).unwrap().len(), 32);
        assert_eq!(
            std::fs::read_to_string(dir.join(TOKENS)).unwrap(),
            "lattice-bearer-tokens v1\n"
        );
        let before = std::fs::read(dir.join(DECOY)).unwrap();
        let error = cmd(&format!("init {d}"), "", 2).unwrap_err();
        assert!(error.contains("never overwrites"), "{error}");
        assert_eq!(std::fs::read(dir.join(DECOY)).unwrap(), before);
        // A directory the owner shares with others is refused before anything is written.
        let open = directory("init-open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(cmd(&format!("init {}", open.display()), "", 1).is_err());
        assert_eq!(std::fs::read_dir(&open).unwrap().count(), 0);
        let out = cmd(
            &format!("init {}", directory("init-flags").display()),
            "",
            3,
        )
        .unwrap();
        for flag in [
            "--auth scram-tls",
            "--scram-verifiers",
            "--scram-decoy-secret",
            "--account-policies",
            "--tls-cert",
            "--tls-key",
        ] {
            assert!(out.contains(flag), "{flag}");
        }
        for path in [dir, open, directory("init-flags")] {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(path);
        }
    }

    #[test]
    fn account_add_writes_a_verifier_the_server_accepts() {
        let (dir, d) = ready("add");
        let out = cmd(
            &format!("add {d} alice --read work_items,docs --write work_items --password-stdin"),
            &format!("{PASSWORD}\n"),
            5,
        )
        .unwrap();
        assert!(!out.contains(PASSWORD), "stdin password echoed");
        assert!(out.contains("grants: tables: docs=r, work_items=rw"));
        assert_eq!(mode(&dir.join(VERIFIERS)), 0o600);
        // The exact line is the independently computed verifier for the salt the generator drew.
        let text = std::fs::read_to_string(dir.join(VERIFIERS)).unwrap();
        let (name, line) = text.trim_end().split_once('\t').unwrap();
        assert_eq!(name, "alice");
        let salt_b64 = line.split(':').nth(1).unwrap().split('$').next().unwrap();
        assert_eq!(salt_b64.len(), 24);
        // What latticed does with the file.
        let decoy = DecoySecret::from_bytes(&std::fs::read(dir.join(DECOY)).unwrap()).unwrap();
        Accounts::parse(&text, decoy).unwrap();
        let policies = std::fs::read_to_string(dir.join(POLICIES)).unwrap();
        assert_eq!(policies, "alice\ttables: docs=r, work_items=rw\n");
        assert!(PolicyTable::parse_accounts(&policies).is_ok());
        // No password and no key material appears in anything printed.
        assert!(!out.contains(line));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_generated_password_is_printed_once_and_authenticates_only_with_that_value() {
        let (dir, d) = ready("generated");
        let out = cmd(&format!("add {d} bob"), "", 9).unwrap();
        let password = out
            .lines()
            .find_map(|l| l.strip_prefix("password (shown once, not stored): "))
            .unwrap();
        assert_eq!(password.len(), 48);
        assert!(password.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(out.contains("grants: deny-all"));
        let text = std::fs::read_to_string(dir.join(VERIFIERS)).unwrap();
        let (_, line) = text.trim_end().split_once('\t').unwrap();
        let salt = lattice_salt(line);
        assert_eq!(&**scram_line(password.as_bytes(), &salt).unwrap(), line);
        assert_ne!(&**scram_line(b"not the password", &salt).unwrap(), line);
        // Neither the file nor the listing holds the password.
        assert!(!text.contains(password));
        assert!(!cmd(&format!("list {d}"), "", 1).unwrap().contains(password));
        let _ = std::fs::remove_dir_all(dir);
    }

    fn lattice_salt(line: &str) -> Vec<u8> {
        use base64::Engine as _;
        let b64 = line.split(':').nth(1).unwrap().split('$').next().unwrap();
        base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap()
    }

    #[test]
    fn account_names_are_validated_like_the_credential_parser() {
        let (dir, d) = ready("names");
        let max = "n".repeat(63);
        let cases: Vec<(String, bool)> = vec![
            ("alice".into(), true),
            (max.clone(), true),
            ("n".repeat(64), false),
            (String::new(), false),
            ("a\tb".into(), false),
            ("a\nb".into(), false),
            ("a\rb".into(), false),
            ("a\u{7f}b".into(), false),
            ("a\u{0}b".into(), false),
            ("a b".into(), true),
        ];
        for (name, ok) in cases {
            // Oracle: the parser latticed uses for the file, fed a line with this name.
            let line = format!("{name}\t{}", &**scram_line(b"pencil", &[7u8; 16]).unwrap());
            let parser_accepts =
                Accounts::parse(&line, DecoySecret::from_bytes(&[1; 32]).unwrap()).is_ok();
            assert_eq!(parser_accepts, ok, "parser {name:?}");
            let args: Vec<String> = vec!["add".into(), d.clone(), name.clone()];
            let mut input = Cursor::new(Vec::new());
            let mut rng = Counter(3);
            let result = run(
                &args,
                &mut Env {
                    stdin: &mut input,
                    rng: &mut rng,
                    now_secs: NOW,
                    stdin_is_terminal: false,
                },
            );
            assert_eq!(result.is_ok(), ok, "tool {name:?}: {result:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn weak_empty_and_non_printable_passwords_are_refused() {
        let (dir, d) = ready("weak");
        for (password, ok) in [
            ("", false),
            ("shortpass11", false),
            ("exactly12chr", true),
            ("pass\tword12345", false),
            ("pässword123456", false),
            (&"x".repeat(257), false),
            (&"x".repeat(256), true),
        ] {
            let name = format!("u{}", password.len());
            let result = cmd(&format!("add {d} {name} --password-stdin"), password, 4);
            assert_eq!(result.is_ok(), ok, "{password:?}: {result:?}");
            if let Err(e) = result {
                assert!(
                    !e.contains(password) || password.is_empty(),
                    "error echoes the password"
                );
            }
        }
        // The planted defect drops the minimum and the test above notices.
        bugs::enable(Bug::WeakPasswordAccepted);
        let weak = cmd(&format!("add {d} weakling --password-stdin"), "pw", 4);
        bugs::clear();
        assert!(
            weak.is_ok(),
            "the planted bug must let a short password through"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn passwords_and_tokens_are_never_accepted_as_arguments() {
        let (dir, d) = ready("argv");
        for line in [
            format!("add {d} carol --password hunter2hunter2"),
            format!("add {d} carol --password=hunter2hunter2"),
            format!("add {d} carol --token abc"),
            format!("token create {d} carol --token abc"),
        ] {
            let error = cmd(&line, "", 1).unwrap_err();
            assert!(!error.contains("hunter2"), "{error}");
            assert!(error.contains("unknown option"), "{error}");
        }
        assert!(!dir.join(VERIFIERS).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_missing_and_invalid_grants_change_nothing() {
        let (dir, d) = ready("dups");
        cmd(&format!("add {d} alice --read docs"), "", 1).unwrap();
        let snapshot =
            |dir: &Path| [VERIFIERS, POLICIES, TOKENS].map(|n| std::fs::read(dir.join(n)).unwrap());
        let before = snapshot(&dir);
        for line in [
            format!("add {d} alice"),
            format!("add {d} dave --read bad=name"),
            format!("add {d} dave --write a b"),
            format!("remove {d} nobody"),
            format!("token create {d} nobody"),
            format!("token revoke {d} 0000000000000000"),
            format!("token revoke {d} NOT-HEX"),
        ] {
            assert!(cmd(&line, "", 2).is_err(), "{line}");
        }
        assert_eq!(snapshot(&dir), before);
        // Wildcard and ddl grants render in the order the parser reads back.
        cmd(
            &format!("add {d} erin --read * --write docs --ddl docs"),
            "",
            3,
        )
        .unwrap();
        let policies = std::fs::read_to_string(dir.join(POLICIES)).unwrap();
        assert!(
            policies.contains("erin\ttables: *=r, docs=wd\n")
                || policies.contains("erin\ttables: docs=wd, *=r\n"),
            "{policies}"
        );
        assert!(PolicyTable::parse_accounts(&policies).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_token_has_at_least_256_bits_and_is_never_stored_in_clear() {
        let (dir, d) = ready("token");
        cmd(&format!("add {d} alice"), "", 1).unwrap();
        let out = cmd(&format!("token create {d} alice"), "", 77).unwrap();
        let token = out
            .lines()
            .find_map(|l| {
                l.split_once("only a salted hash is stored): ")
                    .map(|(_, t)| t)
            })
            .unwrap();
        // lat_<16 hex>_<43 base64url chars = 32 bytes>
        assert_eq!(token.len(), 64);
        // The secret may itself contain `_`: it starts after `lat_` + 16 hex + `_`.
        let secret = &token[21..];
        assert_eq!(secret.len(), 43);
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(secret)
            .unwrap();
        assert_eq!(raw.len(), 32);
        let file = std::fs::read_to_string(dir.join(TOKENS)).unwrap();
        assert!(!file.contains(secret) && !file.contains(token));
        assert!(!file.contains(&*hex(&raw)));
        assert_eq!(mode(&dir.join(TOKENS)), 0o600);
        // The stored hash does verify the token the tool printed (the independent verifier).
        let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
        let header = format!("Bearer {token}");
        let principal = verify_bearer(Some(header.as_bytes()), &store, NOW).unwrap();
        assert_eq!(principal.name(), "alice");
        // Listing shows ids and states, not secrets.
        let listing = cmd(&format!("token list {d}"), "", 1).unwrap();
        assert!(!listing.contains(secret));
        assert!(listing.contains("alice\tactive"));
        // Two tokens differ.
        let again = cmd(&format!("token create {d} alice --rotate"), "", 78).unwrap();
        assert!(!again.contains(secret));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rotation_needs_the_flag_ends_the_older_token_and_allows_only_two() {
        let (dir, d) = ready("rotate");
        cmd(&format!("add {d} alice"), "", 1).unwrap();
        let first = cmd(&format!("token create {d} alice --expires-days 30"), "", 10).unwrap();
        assert!(
            cmd(&format!("token create {d} alice"), "", 11)
                .unwrap_err()
                .contains("--rotate")
        );
        let second = cmd(
            &format!("token create {d} alice --rotate --overlap-hours 2"),
            "",
            12,
        )
        .unwrap();
        assert!(
            cmd(&format!("token create {d} alice --rotate"), "", 13)
                .unwrap_err()
                .contains("two active")
        );
        assert!(cmd(&format!("token create {d} alice --overlap-hours 2"), "", 13).is_err());
        assert!(
            cmd(
                &format!("token create {d} alice --rotate --overlap-hours 169"),
                "",
                13
            )
            .is_err()
        );
        let token_of = |out: &str| {
            out.lines()
                .find_map(|l| l.split_once("stored): ").map(|(_, t)| t.to_string()))
                .unwrap()
        };
        let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
        let check = |token: &str, at: u64| {
            verify_bearer(Some(format!("Bearer {token}").as_bytes()), &store, at)
                .map(|p| p.name().to_string())
        };
        let (a, b) = (token_of(&first), token_of(&second));
        // Both work inside the 2 hour overlap; only the newer after it (the 30 day expiry is later).
        assert_eq!(check(&a, NOW + 7199).unwrap(), "alice");
        assert_eq!(check(&b, NOW + 7199).unwrap(), "alice");
        assert_eq!(
            check(&a, NOW + 7200),
            Err(AuthError::Rejected(RejectReason::Expired))
        );
        assert_eq!(check(&b, NOW + 7200).unwrap(), "alice");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn revoking_a_token_and_removing_an_account_stop_the_credentials() {
        let (dir, d) = ready("revoke");
        cmd(&format!("add {d} alice --read docs"), "", 1).unwrap();
        cmd(&format!("add {d} bob"), "", 2).unwrap();
        let out = cmd(&format!("token create {d} alice"), "", 3).unwrap();
        let token = out
            .lines()
            .find_map(|l| l.split_once("stored): ").map(|(_, t)| t.to_string()))
            .unwrap();
        let id = token.split('_').nth(1).unwrap().to_string();
        let accepted = |dir: &Path| {
            let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
            verify_bearer(Some(format!("Bearer {token}").as_bytes()), &store, NOW).is_ok()
        };
        assert!(accepted(&dir));
        cmd(&format!("token revoke {d} {id}"), "", 4).unwrap();
        assert!(!accepted(&dir));
        // A revoked entry stays in the file so the id is never reissued.
        assert!(
            std::fs::read_to_string(dir.join(TOKENS))
                .unwrap()
                .contains(&id)
        );

        let out = cmd(&format!("token create {d} alice"), "", 5).unwrap();
        let token = out
            .lines()
            .find_map(|l| l.split_once("stored): ").map(|(_, t)| t.to_string()))
            .unwrap();
        let accepted = |dir: &Path| {
            let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
            verify_bearer(Some(format!("Bearer {token}").as_bytes()), &store, NOW).is_ok()
        };
        assert!(accepted(&dir));
        // The planted defect leaves the token alive: the assertion below is what catches it.
        bugs::enable(Bug::AccountRemoveKeepsTokens);
        let kept = cmd(&format!("remove {d} alice"), "", 6);
        bugs::clear();
        assert!(kept.unwrap().contains("0 bearer tokens revoked"));
        assert!(
            accepted(&dir),
            "the planted bug must leave the token working"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn remove_deletes_login_grants_and_tokens_and_the_last_account_removes_the_file() {
        let (dir, d) = ready("remove");
        cmd(&format!("add {d} alice --read docs"), "", 1).unwrap();
        cmd(&format!("add {d} bob"), "", 2).unwrap();
        let out = cmd(&format!("token create {d} alice"), "", 3).unwrap();
        let token = out
            .lines()
            .find_map(|l| l.split_once("stored): ").map(|(_, t)| t.to_string()))
            .unwrap();
        let removed = cmd(&format!("remove {d} alice"), "", 4).unwrap();
        assert!(removed.contains("1 bearer tokens revoked"));
        let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
        assert!(verify_bearer(Some(format!("Bearer {token}").as_bytes()), &store, NOW).is_err());
        let verifiers = std::fs::read_to_string(dir.join(VERIFIERS)).unwrap();
        assert!(!verifiers.contains("alice") && verifiers.contains("bob"));
        assert_eq!(
            std::fs::read_to_string(dir.join(POLICIES)).unwrap(),
            "bob\tdeny-all\n"
        );
        let listing = cmd(&format!("list {d}"), "", 1).unwrap();
        assert!(listing.contains("bob\tscram: yes\ttokens active: 0\tgrants: deny-all"));
        assert!(listing.ends_with("1 accounts\n"));
        cmd(&format!("remove {d} bob"), "", 5).unwrap();
        assert!(!dir.join(VERIFIERS).exists());
        // The name is free again.
        cmd(&format!("add {d} alice"), "", 6).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn commands_refuse_an_uninitialised_or_world_writable_directory() {
        let dir = directory("uninit");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let d = dir.display().to_string();
        let error = cmd(&format!("add {d} alice"), "", 1).unwrap_err();
        assert!(error.contains("account init"), "{error}");
        cmd(&format!("init {d}"), "", 1).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(cmd(&format!("add {d} alice"), "", 1).is_err());
        assert!(cmd(&format!("list {d}"), "", 1).is_err());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A file others can read is refused too.
        std::fs::set_permissions(dir.join(POLICIES), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(cmd(&format!("list {d}"), "", 1).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn help_lists_every_command() {
        let out = cmd("--help", "", 1).unwrap();
        for word in [
            "init",
            "add",
            "list",
            "remove",
            "token create",
            "token revoke",
            "token list",
            "--password-stdin",
        ] {
            assert!(out.contains(word), "{word}");
        }
        assert!(cmd("frobnicate", "", 1).is_err());
    }

    #[test]
    fn grant_table_names_cannot_inject_another_account_line() {
        let (dir, d) = ready("inject");
        // Each name would, unvalidated, end the line and start one for another account (the
        // second line `evil<TAB>tables: *=r` is what an injected policy file would hold).
        for hostile in [
            "x=r\nevil\ttables: *",
            "x\nevil\ttables: *=rwd",
            "x\tevil",
            "x\revil",
            "a b",
            "a=r,b",
            "a,b=",
            "",
            "x\u{0}y",
        ] {
            for flag in ["--read", "--write", "--ddl"] {
                let result = run_at(&["add", &d, "alice", flag, hostile], "", 3, NOW, false);
                assert!(result.is_err(), "{flag} {hostile:?} was accepted");
            }
        }
        // Nothing was written by any refused command.
        assert_eq!(std::fs::read_to_string(dir.join(POLICIES)).unwrap(), "");
        assert!(!dir.join(VERIFIERS).exists());
        // An accepted add leaves exactly one account in the policy file.
        run_at(
            &["add", &d, "alice", "--read", "docs,*", "--write", "docs"],
            "",
            3,
            NOW,
            false,
        )
        .unwrap();
        let policies = std::fs::read_to_string(dir.join(POLICIES)).unwrap();
        assert_eq!(policies.lines().count(), 1);
        let parsed = PolicyTable::parse_accounts(&policies).unwrap();
        assert_eq!(
            parsed.keys().map(String::as_str).collect::<Vec<_>>(),
            ["alice"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_command_waits_for_the_directory_lock_and_the_planted_bug_does_not() {
        use std::sync::mpsc;
        use std::time::Duration;
        let (dir, d) = ready("lock");
        run_at(&["add", &d, "alice"], "", 1, NOW, false).unwrap();
        let held = latticed::files::lock_dir(&dir).unwrap();
        assert_eq!(mode(&dir.join(".lock")), 0o600);
        let (tx, rx) = mpsc::channel();
        let worker = {
            let d = d.clone();
            std::thread::spawn(move || {
                let result = run_at(&["token", "create", &d, "alice"], "", 2, NOW, false);
                tx.send(result.is_ok()).unwrap();
            })
        };
        // Held: the command cannot have finished.
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err());
        drop(held);
        assert!(rx.recv_timeout(Duration::from_secs(20)).unwrap());
        worker.join().unwrap();

        // The planted defect takes no lock: the command finishes while the lock is held.
        let held = latticed::files::lock_dir(&dir).unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = {
            let d = d.clone();
            std::thread::spawn(move || {
                bugs::enable(Bug::AccountNoLock);
                let result = run_at(
                    &["token", "revoke", &d, "0000000000000001"],
                    "",
                    2,
                    NOW,
                    false,
                );
                bugs::clear();
                tx.send(result.is_err()).unwrap();
            })
        };
        assert!(
            rx.recv_timeout(Duration::from_secs(20)).is_ok(),
            "the planted bug must not wait for the lock"
        );
        drop(held);
        worker.join().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_revocation_racing_a_token_creation_is_never_undone() {
        for round in 0..8u64 {
            let (dir, d) = ready(&format!("race{round}"));
            run_at(&["add", &d, "alice"], "", 1, NOW, false).unwrap();
            let out = run_at(&["token", "create", &d, "alice"], "", 2, NOW, false).unwrap();
            let id = out
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let revoke = {
                let (d, id) = (d.clone(), id.clone());
                std::thread::spawn(move || run_at(&["token", "revoke", &d, &id], "", 3, NOW, false))
            };
            let create = {
                let d = d.clone();
                std::thread::spawn(move || {
                    run_at(
                        &["token", "create", &d, "alice", "--rotate"],
                        "",
                        4 + round,
                        NOW,
                        false,
                    )
                })
            };
            revoke.join().unwrap().unwrap();
            create.join().unwrap().unwrap();
            let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
            let first = store.get(TokenId::parse(&id).unwrap()).unwrap();
            assert_eq!(first.status(), TokenStatus::Revoked, "round {round}");
            assert_eq!(store.len(), 2, "round {round}: the new token is there too");
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn init_is_all_or_nothing() {
        let dir = directory("allornothing");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = Dir(dir.clone());
        // The second file cannot be created (its parent does not exist): the first goes again.
        let error = create_all(&target, &[("first", b"a"), ("missing/second", b"b")]).unwrap_err();
        assert!(error.contains("missing"), "{error}");
        assert!(!dir.join("first").exists());
        // A retry from the same directory then completes.
        create_all(&target, &[("first", b"a"), ("second", b"b")]).unwrap();
        assert_eq!(std::fs::read(dir.join("second")).unwrap(), b"b");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_clock_reading_of_zero_issues_no_token() {
        let (dir, d) = ready("clock");
        cmd(&format!("add {d} alice"), "", 1).unwrap();
        for line in [
            format!("token create {d} alice"),
            format!("token prune {d}"),
        ] {
            let error = cmd_at(&line, "", 2, 0).unwrap_err();
            assert!(error.contains("clock"), "{error}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.join(TOKENS)).unwrap(),
            "lattice-bearer-tokens v1\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn expired_tokens_free_their_slot_show_as_expired_and_prune_removes_them() {
        let (dir, d) = ready("expired");
        cmd(&format!("add {d} alice"), "", 1).unwrap();
        cmd(&format!("token create {d} alice --expires-days 1"), "", 2).unwrap();
        let day = 86_400;
        // Two days later the first token is expired: it is not "active" in list or token list,
        // and a new token needs no --rotate.
        let later = NOW + 2 * day;
        assert!(
            cmd_at(&format!("list {d}"), "", 1, later)
                .unwrap()
                .contains("tokens active: 0")
        );
        assert!(
            cmd_at(&format!("token list {d}"), "", 1, later)
                .unwrap()
                .contains("alice\texpired")
        );
        cmd_at(&format!("token create {d} alice"), "", 3, later).unwrap();
        // The expired entry was revoked; it is only one day past its expiry, so a 30 day prune
        // keeps it and a 0 day prune removes it and keeps the live token.
        assert_eq!(
            latticed::load_credential_store(&dir.join(TOKENS))
                .unwrap()
                .len(),
            2
        );
        let out = cmd_at(&format!("token prune {d}"), "", 1, later).unwrap();
        assert!(out.starts_with("0 dead tokens removed"), "{out}");
        let out = cmd_at(
            &format!("token prune {d} --older-than-days 0"),
            "",
            1,
            later,
        )
        .unwrap();
        assert!(
            out.starts_with("1 dead tokens removed, 1 entries remain"),
            "{out}"
        );
        let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
        assert_eq!(
            store
                .entries()
                .filter(|e| e.status() == TokenStatus::Active)
                .count(),
            1
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_full_token_file_prunes_old_dead_entries_instead_of_failing_forever() {
        let (dir, d) = ready("full");
        cmd(&format!("add {d} alice"), "", 1).unwrap();
        let mut rng = Counter(99);
        let mut store = CredentialStore::empty();
        for _ in 0..lattice_api::bearer::MAX_ENTRIES {
            let (_, entry) = issue("alice", 10, None, &mut rng).unwrap();
            let id = entry.id();
            store.insert(entry).unwrap();
            store.revoke(id);
        }
        latticed::write_credential_store(&dir.join(TOKENS), &store).unwrap();
        cmd(&format!("token create {d} alice"), "", 5).unwrap();
        let store = latticed::load_credential_store(&dir.join(TOKENS)).unwrap();
        assert_eq!(store.len(), 1);
        // Recent dead entries are not pruned behind the user's back: the command says what to run.
        let mut recent = CredentialStore::empty();
        for _ in 0..lattice_api::bearer::MAX_ENTRIES {
            let (_, entry) = issue("alice", NOW - 5, None, &mut rng).unwrap();
            let id = entry.id();
            recent.insert(entry).unwrap();
            recent.revoke(id);
        }
        latticed::write_credential_store(&dir.join(TOKENS), &recent).unwrap();
        let error = cmd(&format!("token create {d} alice"), "", 6).unwrap_err();
        assert!(error.contains("token prune"), "{error}");
        cmd(&format!("token prune {d} --older-than-days 0"), "", 6).unwrap();
        cmd(&format!("token create {d} alice"), "", 7).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_terminal_stdin_is_refused_for_the_password() {
        let (dir, d) = ready("tty");
        let error = run_at(
            &["add", &d, "alice", "--password-stdin"],
            "a long enough password\n",
            1,
            NOW,
            true,
        )
        .unwrap_err();
        assert!(error.contains("terminal"), "{error}");
        assert!(!dir.join(VERIFIERS).exists());
        // A generated password needs no stdin, terminal or not.
        run_at(&["add", &d, "alice"], "", 1, NOW, true).unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
